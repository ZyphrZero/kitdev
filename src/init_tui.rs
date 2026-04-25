use std::{
    io,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
};

use crate::{
    config::DevkitConfig,
    doctor::{DoctorReport, Status, build_doctor_report},
    init::{
        CliDraft, GoDraft, HomebrewDraft, InitDraft, InitInteractionOutcome, InitWriteResult,
        NodeDraft, NpmDraft, RustDraft, SimpleToolDraft, render_init_document, write_init_document,
    },
    latest::{VersionCandidate, VersionCandidates, lookup_version_candidates},
    platform::OperatingSystem,
    sync::{
        SyncExecution, SyncPlan, SyncStepExecutionStatus, SyncStepKind, build_sync_plan,
        execute_sync_plan,
    },
};

const TOOL_NAMES: &[&str] = &[
    "fnm", "nvm", "node", "npm", "pnpm", "yarn", "bun", "deno", "go", "rust", "uv", "python",
    "poetry", "ruby", "wrangler",
];

const MENU_LEN: usize = TOOL_NAMES.len() + 5;
const ENABLED_DOT: &str = "●";
const DISABLED_DOT: &str = "○";

#[derive(Debug, Clone)]
pub struct InitTuiOptions {
    pub output: PathBuf,
    pub force: bool,
    pub stdout: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Menu,
    Fields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuEntry {
    Policy,
    Tool(&'static str),
    Homebrew,
    Npm,
    Actions,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionTarget {
    SaveConfig,
    RunCheck,
    PreviewSync,
    ApplySync,
    Finish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldTarget {
    PolicyChannel,
    PolicyPlatform,
    ToolEnabled(&'static str),
    SimpleManager(&'static str),
    NodeVersion,
    NodeManager,
    NodePackageManagers,
    GoVersion,
    GoManager,
    GoSource,
    GoInstallDir,
    RustChannel,
    HomebrewPackages,
    NpmGlobals,
    Action(ActionTarget),
}

#[derive(Debug, Clone)]
struct FieldRow {
    label: String,
    value: String,
    target: FieldTarget,
}

#[derive(Debug, Clone)]
struct EditState {
    target: FieldTarget,
    label: String,
    buffer: String,
}

struct VersionFetchState {
    target: FieldTarget,
    label: String,
    current: String,
    receiver: Receiver<VersionCandidates>,
}

#[derive(Debug, Clone)]
struct VersionPickerState {
    target: FieldTarget,
    label: String,
    source: String,
    note: Option<String>,
    choices: Vec<VersionCandidate>,
    selected: usize,
    custom_buffer: String,
    custom_mode: bool,
}

struct ActionTaskState {
    title: String,
    receiver: Receiver<std::result::Result<ActionOutput, String>>,
}

#[derive(Debug, Clone)]
struct ActionOutput {
    title: String,
    lines: Vec<String>,
    ok: bool,
    mark_saved: bool,
}

#[derive(Debug, Clone)]
struct ConfirmState {
    action: ActionTarget,
    title: String,
    message: String,
}

struct InitTuiApp {
    draft: InitDraft,
    options: InitTuiOptions,
    menu_index: usize,
    field_index: usize,
    focus: Focus,
    edit: Option<EditState>,
    version_fetch: Option<VersionFetchState>,
    version_picker: Option<VersionPickerState>,
    action_task: Option<ActionTaskState>,
    action_output: Option<ActionOutput>,
    action_scroll: u16,
    confirm: Option<ConfirmState>,
    status: String,
    preview_scroll: u16,
    preview_expanded: bool,
    action_expanded: bool,
    saved_once: bool,
    dirty: bool,
}

enum AppExit {
    Continue,
    Handled,
    Cancelled,
}

pub fn customize_init_draft_tui(
    draft: &mut InitDraft,
    options: InitTuiOptions,
) -> Result<InitInteractionOutcome> {
    enable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_tui(&mut terminal, draft, options);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    draft: &mut InitDraft,
    options: InitTuiOptions,
) -> Result<InitInteractionOutcome> {
    let mut app = InitTuiApp::with_options(draft.clone(), options);
    let mut last_tick = Instant::now();

    loop {
        app.poll_version_fetch();
        app.poll_action_task();
        terminal.draw(|frame| app.render(frame))?;

        let timeout = Duration::from_millis(250)
            .checked_sub(last_tick.elapsed())
            .unwrap_or_default();
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && let Some(exit) = app.handle_key(key)
        {
            match exit {
                AppExit::Continue => {
                    *draft = app.draft;
                    return Ok(InitInteractionOutcome::Continue);
                }
                AppExit::Handled => {
                    *draft = app.draft;
                    return Ok(InitInteractionOutcome::Handled);
                }
                AppExit::Cancelled => return Ok(InitInteractionOutcome::Cancelled),
            }
        }

        if last_tick.elapsed() >= Duration::from_millis(250) {
            last_tick = Instant::now();
        }
    }
}

impl InitTuiApp {
    #[cfg(test)]
    fn new(draft: InitDraft) -> Self {
        Self::with_options(
            draft,
            InitTuiOptions {
                output: PathBuf::from("devkit.toml"),
                force: false,
                stdout: false,
            },
        )
    }

    fn with_options(draft: InitDraft, options: InitTuiOptions) -> Self {
        Self {
            draft,
            options,
            menu_index: 0,
            field_index: 0,
            focus: Focus::Menu,
            edit: None,
            version_fetch: None,
            version_picker: None,
            action_task: None,
            action_output: None,
            action_scroll: 0,
            confirm: None,
            status: "Ready. Edit the policy, then use Actions to check or sync.".to_string(),
            preview_scroll: 0,
            preview_expanded: false,
            action_expanded: false,
            saved_once: false,
            dirty: true,
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let root = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(14),
                Constraint::Length(4),
            ])
            .split(area);

        self.render_header(frame, root[0]);
        self.render_body(frame, root[1]);
        self.render_footer(frame, root[2]);

        if let Some(edit) = &self.edit {
            render_edit_popup(frame, area, edit);
        }
        if let Some(fetch) = &self.version_fetch {
            render_version_loading_popup(frame, area, fetch);
        }
        if let Some(picker) = &self.version_picker {
            render_version_picker(frame, area, picker);
        }
        if let Some(confirm) = &self.confirm {
            render_confirm_popup(frame, area, confirm);
        }
        if let Some(task) = &self.action_task {
            render_action_loading_popup(frame, area, task);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let enabled = TOOL_NAMES
            .iter()
            .filter(|tool| tool_enabled(&self.draft, tool))
            .count();
        let state = if self.action_task.is_some() {
            ("running", Color::Yellow)
        } else if self.dirty {
            ("unsaved", Color::Yellow)
        } else {
            ("saved", Color::Green)
        };
        let output = if self.options.stdout {
            "stdout".to_string()
        } else {
            self.options.output.display().to_string()
        };
        let content = vec![
            Line::from(vec![
                Span::styled(
                    "devkit init",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  policy builder  "),
                Span::styled(
                    format!("[{}]", state.0),
                    Style::default().fg(state.1).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::raw("channel "),
                Span::styled(
                    &self.draft.policy.channel,
                    Style::default().fg(Color::Green),
                ),
                Span::raw("  platform "),
                Span::styled(
                    &self.draft.policy.platform,
                    Style::default().fg(Color::Green),
                ),
                Span::raw(format!("  enabled tools {enabled}/{}", TOOL_NAMES.len())),
                Span::raw("  output "),
                Span::styled(output, Style::default().fg(Color::Magenta)),
            ]),
        ];
        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .padding(Padding::horizontal(1)),
            )
            .alignment(Alignment::Left);
        frame.render_widget(paragraph, area);
    }

    fn render_body(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);

        if self.preview_expanded {
            self.render_preview(frame, area, true);
            return;
        }
        if self.action_expanded {
            self.render_action_output(frame, area, true);
            return;
        }

        if area.width >= 112 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(31),
                    Constraint::Length(44),
                    Constraint::Min(37),
                ])
                .split(area);

            self.render_menu(frame, chunks[0]);
            self.render_fields(frame, chunks[1]);
            self.render_side_panel(frame, chunks[2]);
        } else {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
                .split(area);
            let top = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(28), Constraint::Min(34)])
                .split(rows[0]);

            self.render_menu(frame, top[0]);
            self.render_fields(frame, top[1]);
            self.render_side_panel(frame, rows[1]);
        }
    }

    fn render_menu(&self, frame: &mut Frame, area: Rect) {
        let items = menu_entries()
            .into_iter()
            .map(|entry| ListItem::new(menu_line(&self.draft, entry)))
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        state.select(Some(self.menu_index));
        let border = if self.focus == Focus::Menu {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let list = List::new(items)
            .block(
                Block::default()
                    .title(" Sections ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
                    .padding(Padding::horizontal(1)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn render_fields(&mut self, frame: &mut Frame, area: Rect) {
        let entry = current_menu_entry(self.menu_index);
        let fields = field_rows(&self.draft, entry, &self.options);
        if self.field_index >= fields.len() {
            self.field_index = fields.len().saturating_sub(1);
        }

        let accent = entry_accent(entry);
        let border = if self.focus == Focus::Fields {
            accent
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .title(format!(" {} ", entry_title(entry)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .padding(Padding::horizontal(1));

        if fields.is_empty() {
            let text = match entry {
                MenuEntry::Actions => {
                    "Run actions from this TUI: save the policy, check the machine, preview sync, apply sync, or finish."
                }
                MenuEntry::Preview => {
                    "Press Enter or P to open the full TOML preview.\n\nPress S to accept this policy or Q to cancel."
                }
                _ => "No editable fields for this section.",
            };
            frame.render_widget(
                Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
                area,
            );
            return;
        }

        let items = fields
            .iter()
            .map(|field| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<18}", field.label),
                        field_label_style(&field.target),
                    ),
                    Span::styled(
                        field.value.clone(),
                        field_value_style(&field.target, &field.value),
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        state.select(Some(self.field_index));
        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(accent)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn render_side_panel(&self, frame: &mut Frame, area: Rect) {
        if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
            self.render_action_output(frame, area, false);
        } else {
            self.render_preview(frame, area, false);
        }
    }

    fn render_preview(&self, frame: &mut Frame, area: Rect, expanded: bool) {
        let preview = render_init_document(&self.draft).content;
        let title = if expanded {
            " Preview - full (P/Tab back, PgUp/PgDn scroll) "
        } else {
            " Preview "
        };
        let paragraph = Paragraph::new(preview)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .scroll((self.preview_scroll, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_action_output(&self, frame: &mut Frame, area: Rect, expanded: bool) {
        let output_label = if self.options.stdout {
            "stdout preview".to_string()
        } else {
            self.options.output.to_string_lossy().to_string()
        };
        let (title, lines, ok) = match &self.action_output {
            Some(output) => {
                let title = if expanded {
                    format!(
                        " Actions - full - {} (P/Tab back, PgUp/PgDn scroll) ",
                        output.title
                    )
                } else {
                    format!(" Actions - {} ", output.title)
                };
                (title, output.lines.clone(), output.ok)
            }
            None => (
                if expanded {
                    " Actions - full (P/Tab back) ".to_string()
                } else {
                    " Actions ".to_string()
                },
                vec![
                    format!("Output: {output_label}"),
                    String::new(),
                    "Choose an action in the center pane.".to_string(),
                ],
                true,
            ),
        };
        let border = if ok { Color::DarkGray } else { Color::Red };
        let paragraph = Paragraph::new(lines.join("\n"))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border)),
            )
            .scroll((self.action_scroll, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let keys = if self.preview_expanded || self.action_expanded {
            Line::from(vec![
                Span::styled("Up/Dn", Style::default().fg(Color::Cyan)),
                Span::raw(" scroll  "),
                Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
                Span::raw(" page  "),
                Span::styled("P/Tab", Style::default().fg(Color::Cyan)),
                Span::raw(" back  "),
                Span::styled("S", Style::default().fg(Color::Green)),
                Span::raw(" save  "),
                Span::styled("F", Style::default().fg(Color::Green)),
                Span::raw(" done  "),
                Span::styled("Q", Style::default().fg(Color::Red)),
                Span::raw(" quit"),
            ])
        } else if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
            Line::from(vec![
                Span::styled("Up/Dn", Style::default().fg(Color::Cyan)),
                Span::raw(" move  "),
                Span::styled("L/R", Style::default().fg(Color::Cyan)),
                Span::raw(" pane  "),
                Span::styled("Enter", Style::default().fg(Color::Cyan)),
                Span::raw(" run  "),
                Span::styled("P", Style::default().fg(Color::Magenta)),
                Span::raw(" full output  "),
                Span::styled("S", Style::default().fg(Color::Green)),
                Span::raw(" save  "),
                Span::styled("F", Style::default().fg(Color::Green)),
                Span::raw(" done  "),
                Span::styled("Q", Style::default().fg(Color::Red)),
                Span::raw(" quit"),
            ])
        } else {
            Line::from(vec![
                Span::styled("Up/Dn", Style::default().fg(Color::Cyan)),
                Span::raw(" move  "),
                Span::styled("L/R", Style::default().fg(Color::Cyan)),
                Span::raw(" pane  "),
                Span::styled("Space", Style::default().fg(Color::Cyan)),
                Span::raw(" toggle  "),
                Span::styled("Enter", Style::default().fg(Color::Cyan)),
                Span::raw(" edit  "),
                Span::styled("P", Style::default().fg(Color::Magenta)),
                Span::raw(" view  "),
                Span::styled("S", Style::default().fg(Color::Green)),
                Span::raw(" save  "),
                Span::styled("F", Style::default().fg(Color::Green)),
                Span::raw(" done  "),
                Span::styled("Q", Style::default().fg(Color::Red)),
                Span::raw(" quit"),
            ])
        };
        let paragraph = Paragraph::new(vec![keys, Line::from(self.status.as_str())]).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(paragraph, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if self.action_task.is_some() {
                self.status = "An action is running; wait for it to finish.".to_string();
                return None;
            }
            return Some(AppExit::Cancelled);
        }

        if self.confirm.is_some() {
            return self.handle_confirm_key(key);
        }

        if self.action_task.is_some() {
            self.status = "An action is running; wait for it to finish.".to_string();
            return None;
        }

        if self.version_picker.is_some() {
            return self.handle_version_picker_key(key);
        }
        if self.version_fetch.is_some() {
            return self.handle_version_loading_key(key);
        }
        if self.edit.is_some() {
            return self.handle_edit_key(key);
        }

        if self.preview_expanded {
            return self.handle_preview_key(key);
        }
        if self.action_expanded {
            return self.handle_action_output_key(key);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Some(AppExit::Cancelled),
            KeyCode::Esc => {
                self.status =
                    "Use Q to quit; Esc is ignored to protect arrow-key prefixes.".to_string();
                None
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.start_save_action();
                None
            }
            KeyCode::Char('f') | KeyCode::Char('F') => self.finish(),
            KeyCode::Tab => {
                self.toggle_focus();
                None
            }
            KeyCode::Right => {
                self.focus = Focus::Fields;
                None
            }
            KeyCode::Left => {
                self.focus = Focus::Menu;
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            KeyCode::PageUp => {
                self.scroll_active_panel(-8);
                None
            }
            KeyCode::PageDown => {
                self.scroll_active_panel(8);
                None
            }
            KeyCode::Char(' ') => self.toggle_current_tool_or_field(),
            KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('E') => {
                self.start_edit_or_focus_fields()
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
                    self.open_action_output();
                } else {
                    self.open_preview();
                }
                None
            }
            KeyCode::Char('?') => {
                self.status = "P opens full preview; PageUp/PageDown scrolls TOML".to_string();
                None
            }
            _ => None,
        }
    }

    fn handle_preview_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Some(AppExit::Cancelled),
            KeyCode::Esc => {
                self.status = "Use P or Tab to return, Q to quit.".to_string();
                None
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.start_save_action();
                None
            }
            KeyCode::Char('f') | KeyCode::Char('F') => self.finish(),
            KeyCode::Char('p')
            | KeyCode::Char('P')
            | KeyCode::Tab
            | KeyCode::Left
            | KeyCode::Right => {
                self.close_preview();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_preview(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_preview(1);
                None
            }
            KeyCode::PageUp => {
                self.scroll_preview(-12);
                None
            }
            KeyCode::PageDown => {
                self.scroll_preview(12);
                None
            }
            KeyCode::Home => {
                self.preview_scroll = 0;
                None
            }
            KeyCode::End => {
                self.preview_scroll = preview_line_count(&self.draft).saturating_sub(1);
                None
            }
            _ => None,
        }
    }

    fn handle_action_output_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Some(AppExit::Cancelled),
            KeyCode::Esc => {
                self.status = "Use P or Tab to return, Q to quit.".to_string();
                None
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.start_save_action();
                None
            }
            KeyCode::Char('f') | KeyCode::Char('F') => self.finish(),
            KeyCode::Char('p')
            | KeyCode::Char('P')
            | KeyCode::Tab
            | KeyCode::Left
            | KeyCode::Right => {
                self.close_action_output();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_action_output(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_action_output(1);
                None
            }
            KeyCode::PageUp => {
                self.scroll_action_output(-12);
                None
            }
            KeyCode::PageDown => {
                self.scroll_action_output(12);
                None
            }
            KeyCode::Home => {
                self.action_scroll = 0;
                None
            }
            KeyCode::End => {
                self.action_scroll = self.action_output_max_scroll();
                None
            }
            _ => None,
        }
    }

    fn handle_version_loading_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.version_fetch = None;
                self.status = "Version lookup cancelled".to_string();
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if let Some(fetch) = self.version_fetch.take() {
                    self.version_picker = Some(VersionPickerState::custom_only(
                        fetch.target,
                        fetch.label,
                        fetch.current,
                    ));
                    self.status = "Enter a custom version selector".to_string();
                }
            }
            _ => {}
        }
        None
    }

    fn handle_version_picker_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        let mut picker = self.version_picker.take().expect("version picker checked");

        if picker.custom_mode {
            match key.code {
                KeyCode::Esc => {
                    self.status = "Use Ctrl+G to close this edit; Esc is ignored.".to_string();
                    self.version_picker = Some(picker);
                }
                KeyCode::Char('g') | KeyCode::Char('G')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if picker.choices.is_empty() {
                        self.status = "Version edit cancelled".to_string();
                    } else {
                        picker.custom_mode = false;
                        self.status = "Returned to version list".to_string();
                        self.version_picker = Some(picker);
                    }
                }
                KeyCode::Enter => {
                    let value = picker.custom_buffer.trim().to_string();
                    if value.is_empty() {
                        self.status = "Custom version cannot be empty".to_string();
                        self.version_picker = Some(picker);
                    } else {
                        apply_field_edit(&mut self.draft, &picker.target, &value);
                        self.mark_dirty();
                        self.status = format!("Updated {} to {value}", picker.label);
                    }
                }
                KeyCode::Backspace => {
                    picker.custom_buffer.pop();
                    self.version_picker = Some(picker);
                }
                KeyCode::Delete => {
                    picker.custom_buffer.clear();
                    self.version_picker = Some(picker);
                }
                KeyCode::Char(character) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        picker.custom_buffer.push(character);
                    }
                    self.version_picker = Some(picker);
                }
                _ => {
                    self.version_picker = Some(picker);
                }
            }
            return None;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.status = "Version edit cancelled".to_string();
            }
            KeyCode::Esc => {
                self.status = "Use Q to close the version picker".to_string();
                self.version_picker = Some(picker);
            }
            KeyCode::Enter => {
                if let Some(choice) = picker.choices.get(picker.selected) {
                    apply_field_edit(&mut self.draft, &picker.target, &choice.value);
                    self.mark_dirty();
                    self.status = format!("Updated {} to {}", picker.label, choice.value);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                picker.selected = move_index(picker.selected, picker.choices.len(), -1);
                self.version_picker = Some(picker);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                picker.selected = move_index(picker.selected, picker.choices.len(), 1);
                self.version_picker = Some(picker);
            }
            KeyCode::PageUp => {
                picker.selected = move_index(picker.selected, picker.choices.len(), -8);
                self.version_picker = Some(picker);
            }
            KeyCode::PageDown => {
                picker.selected = move_index(picker.selected, picker.choices.len(), 8);
                self.version_picker = Some(picker);
            }
            KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Char('e') | KeyCode::Char('E') => {
                picker.custom_mode = true;
                self.version_picker = Some(picker);
            }
            KeyCode::Char(character) if is_version_character(character) => {
                picker.custom_mode = true;
                picker.custom_buffer.clear();
                picker.custom_buffer.push(character);
                self.version_picker = Some(picker);
            }
            _ => {
                self.version_picker = Some(picker);
            }
        }
        None
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        let mut edit = self.edit.take().expect("edit mode checked");
        match key.code {
            KeyCode::Esc => {
                self.status = "Use Enter to apply this edit; Esc is ignored.".to_string();
                self.edit = Some(edit);
            }
            KeyCode::Char('g') | KeyCode::Char('G')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.status = "Edit cancelled".to_string();
            }
            KeyCode::Enter => {
                apply_field_edit(&mut self.draft, &edit.target, edit.buffer.trim());
                self.mark_dirty();
                self.status = format!("Updated {}", edit.label);
            }
            KeyCode::Backspace => {
                edit.buffer.pop();
                self.edit = Some(edit);
            }
            KeyCode::Delete => {
                edit.buffer.clear();
                self.edit = Some(edit);
            }
            KeyCode::Char(character) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    edit.buffer.push(character);
                }
                self.edit = Some(edit);
            }
            _ => {
                self.edit = Some(edit);
            }
        }
        None
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Menu => Focus::Fields,
            Focus::Fields => Focus::Menu,
        };
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            Focus::Menu => {
                self.menu_index = move_index(self.menu_index, MENU_LEN, delta);
                self.field_index = 0;
            }
            Focus::Fields => {
                let fields = field_rows(
                    &self.draft,
                    current_menu_entry(self.menu_index),
                    &self.options,
                );
                if !fields.is_empty() {
                    self.field_index = move_index(self.field_index, fields.len(), delta);
                }
            }
        }
    }

    fn toggle_current_tool_or_field(&mut self) -> Option<AppExit> {
        match self.focus {
            Focus::Menu => {
                if let MenuEntry::Tool(tool) = current_menu_entry(self.menu_index) {
                    toggle_tool(&mut self.draft, tool);
                    self.mark_dirty();
                    self.status =
                        format!("{tool} {}", enabled_label(tool_enabled(&self.draft, tool)));
                }
            }
            Focus::Fields => {
                if let Some(target) = self.current_field_target() {
                    match target {
                        FieldTarget::ToolEnabled(tool) => {
                            toggle_tool(&mut self.draft, tool);
                            self.mark_dirty();
                            self.status = format!(
                                "{tool} {}",
                                enabled_label(tool_enabled(&self.draft, tool))
                            );
                        }
                        FieldTarget::Action(action) => {
                            return self.run_action(action);
                        }
                        _ => {}
                    }
                }
            }
        }
        None
    }

    fn start_edit_or_focus_fields(&mut self) -> Option<AppExit> {
        if self.focus == Focus::Menu {
            match current_menu_entry(self.menu_index) {
                MenuEntry::Preview => self.open_preview(),
                _ => {
                    self.focus = Focus::Fields;
                    if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
                        self.status = "Select an action and press Enter.".to_string();
                    }
                }
            }
            return None;
        }

        let field = self.current_field()?;
        match field.target {
            FieldTarget::ToolEnabled(tool) => {
                toggle_tool(&mut self.draft, tool);
                self.mark_dirty();
                self.status = format!("{tool} {}", enabled_label(tool_enabled(&self.draft, tool)));
                return None;
            }
            FieldTarget::Action(action) => {
                return self.run_action(action);
            }
            _ => {}
        }

        if version_target_tool(&field.target).is_some() {
            self.start_version_fetch(field);
            return None;
        }

        self.edit = Some(EditState {
            target: field.target,
            label: field.label,
            buffer: field.value,
        });
        None
    }

    fn open_preview(&mut self) {
        self.preview_expanded = true;
        self.action_expanded = false;
        self.status = "Full preview: arrows scroll, PgUp/PgDn page, P returns".to_string();
    }

    fn close_preview(&mut self) {
        self.preview_expanded = false;
        self.status = "Returned to editor. P opens full preview again.".to_string();
    }

    fn open_action_output(&mut self) {
        self.action_expanded = true;
        self.preview_expanded = false;
        self.menu_index = action_menu_index();
        self.status = "Full action output: arrows scroll, PgUp/PgDn page, P returns".to_string();
    }

    fn close_action_output(&mut self) {
        self.action_expanded = false;
        self.status = "Returned to actions. P opens full output again.".to_string();
    }

    fn run_action(&mut self, action: ActionTarget) -> Option<AppExit> {
        match action {
            ActionTarget::SaveConfig => {
                self.start_save_action();
                None
            }
            ActionTarget::RunCheck => {
                self.start_action_task("Check environment", |draft, options| {
                    let config = config_from_draft(&draft)?;
                    let report = build_doctor_report(&config);
                    Ok(doctor_output(report, &options))
                });
                None
            }
            ActionTarget::PreviewSync => {
                self.start_action_task("Preview sync", |draft, options| {
                    let config = config_from_draft(&draft)?;
                    let report = build_doctor_report(&config);
                    let plan = build_sync_plan(true, &options.output, &config, &report);
                    Ok(sync_plan_output(plan))
                });
                None
            }
            ActionTarget::ApplySync => {
                if self.action_task.is_some() {
                    self.status = "An action is already running.".to_string();
                } else {
                    self.confirm = Some(ConfirmState {
                        action,
                        title: "Apply sync".to_string(),
                        message: "This can run install commands and update managed shell snippets."
                            .to_string(),
                    });
                    self.status = "Confirm apply sync from the popup.".to_string();
                }
                None
            }
            ActionTarget::Finish => self.finish(),
        }
    }

    fn start_save_action(&mut self) {
        let allow_overwrite = self.options.force || self.saved_once;
        self.start_action_task("Save config", move |draft, options| {
            save_config_output(&draft, &options, allow_overwrite)
        });
    }

    fn start_apply_sync_action(&mut self) {
        self.start_action_task("Apply sync", |draft, options| {
            let config = config_from_draft(&draft)?;
            let report = build_doctor_report(&config);
            let plan = build_sync_plan(false, &options.output, &config, &report);
            let execution = execute_sync_plan(&plan, &config);
            Ok(sync_execution_output(execution))
        });
    }

    fn start_action_task<F>(&mut self, title: &'static str, work: F)
    where
        F: FnOnce(InitDraft, InitTuiOptions) -> Result<ActionOutput> + Send + 'static,
    {
        if self.action_task.is_some() {
            self.status = "An action is already running.".to_string();
            return;
        }
        self.menu_index = action_menu_index();
        self.focus = Focus::Fields;
        self.preview_expanded = false;
        self.action_expanded = false;
        self.action_scroll = 0;
        let draft = self.draft.clone();
        let options = self.options.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = work(draft, options).map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.action_task = Some(ActionTaskState {
            title: title.to_string(),
            receiver,
        });
        self.status = format!("{title} running...");
    }

    fn poll_action_task(&mut self) {
        let Some(task) = &self.action_task else {
            return;
        };
        let Ok(result) = task.receiver.try_recv() else {
            return;
        };
        let title = self.action_task.take().expect("action task checked").title;
        match result {
            Ok(output) => {
                if output.mark_saved {
                    self.saved_once = true;
                    self.dirty = false;
                }
                self.status = if output.ok {
                    format!("{title} finished")
                } else {
                    format!("{title} finished with issues")
                };
                self.action_output = Some(output);
            }
            Err(error) => {
                self.status = format!("{title} failed");
                self.action_output = Some(ActionOutput::error(title, error));
            }
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        let confirm = self.confirm.take().expect("confirm checked");
        match key.code {
            KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                if confirm.action == ActionTarget::ApplySync {
                    self.start_apply_sync_action();
                }
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.status = format!("{} cancelled", confirm.title);
            }
            KeyCode::Esc => {
                self.status = "Use A to apply or Q to cancel; Esc is ignored.".to_string();
                self.confirm = Some(confirm);
            }
            _ => {
                self.confirm = Some(confirm);
            }
        }
        None
    }

    fn finish(&mut self) -> Option<AppExit> {
        if self.action_task.is_some() {
            self.status = "An action is running; wait for it to finish.".to_string();
            return None;
        }
        if self.options.stdout {
            return Some(AppExit::Continue);
        }
        if !self.dirty && self.saved_once {
            return Some(AppExit::Handled);
        }

        match save_config_output(
            &self.draft,
            &self.options,
            self.options.force || self.saved_once,
        ) {
            Ok(output) => {
                self.saved_once = true;
                self.dirty = false;
                self.action_output = Some(output);
                Some(AppExit::Handled)
            }
            Err(error) => {
                self.menu_index = action_menu_index();
                self.focus = Focus::Fields;
                self.preview_expanded = false;
                self.action_expanded = false;
                self.action_output =
                    Some(ActionOutput::error("Finish".to_string(), error.to_string()));
                self.status = "Finish failed; fix the output path or use --force.".to_string();
                None
            }
        }
    }

    fn start_version_fetch(&mut self, field: FieldRow) {
        let Some(tool) = version_target_tool(&field.target) else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(lookup_version_candidates(tool));
        });
        self.status = format!("Loading {tool} versions...");
        self.version_fetch = Some(VersionFetchState {
            target: field.target,
            label: field.label,
            current: field.value,
            receiver,
        });
    }

    fn poll_version_fetch(&mut self) {
        let Some(fetch) = &self.version_fetch else {
            return;
        };
        let Ok(candidates) = fetch.receiver.try_recv() else {
            return;
        };
        let fetch = self.version_fetch.take().expect("version fetch checked");
        let source = candidates.source.clone();
        let is_empty = candidates.candidates.is_empty();
        self.version_picker = Some(VersionPickerState::new(
            fetch.target,
            fetch.label,
            fetch.current,
            candidates,
        ));
        self.status = if is_empty {
            "No remote versions loaded; enter a custom selector".to_string()
        } else {
            format!("Loaded versions from {source}")
        };
    }

    fn scroll_preview(&mut self, delta: i16) {
        if delta.is_negative() {
            self.preview_scroll = self.preview_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.preview_scroll = self
                .preview_scroll
                .saturating_add(delta as u16)
                .min(preview_line_count(&self.draft).saturating_sub(1));
        }
    }

    fn scroll_active_panel(&mut self, delta: i16) {
        if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
            self.scroll_action_output(delta);
        } else {
            self.scroll_preview(delta);
        }
    }

    fn scroll_action_output(&mut self, delta: i16) {
        let max = self.action_output_max_scroll();
        if delta.is_negative() {
            self.action_scroll = self.action_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.action_scroll = self.action_scroll.saturating_add(delta as u16).min(max);
        }
    }

    fn action_output_max_scroll(&self) -> u16 {
        self.action_output
            .as_ref()
            .map(|output| output.lines.len().saturating_sub(1).min(u16::MAX as usize) as u16)
            .unwrap_or_default()
    }

    fn current_field(&self) -> Option<FieldRow> {
        field_rows(
            &self.draft,
            current_menu_entry(self.menu_index),
            &self.options,
        )
        .get(self.field_index)
        .cloned()
    }

    fn current_field_target(&self) -> Option<FieldTarget> {
        self.current_field().map(|field| field.target)
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

fn render_edit_popup(frame: &mut Frame, area: Rect, edit: &EditState) {
    let popup = centered_rect(62, 9, area);
    frame.render_widget(Clear, popup);
    let content = vec![
        Line::from(Span::styled(
            edit.label.as_str(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(edit.buffer.as_str()),
        Line::from(""),
        Line::from("Enter save   Ctrl+G cancel   Backspace delete"),
    ];
    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Edit ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup);
}

fn render_version_loading_popup(frame: &mut Frame, area: Rect, fetch: &VersionFetchState) {
    let popup = centered_rect(68, 9, area);
    frame.render_widget(Clear, popup);
    let content = vec![
        Line::from(Span::styled(
            format!("Loading versions for {}", fetch.label),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("Current value: {}", fetch.current)),
        Line::from(""),
        Line::from("Press C to type a custom selector now, or Q to cancel."),
    ];
    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Versions ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, popup);
}

fn render_version_picker(frame: &mut Frame, area: Rect, picker: &VersionPickerState) {
    let popup = centered_rect(76, 18, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(format!(" Version - {} ", picker.label))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(inner);

    let mut header = vec![Line::from(vec![
        Span::raw("Source: "),
        Span::styled(picker.source.as_str(), Style::default().fg(Color::Green)),
    ])];
    if let Some(note) = &picker.note {
        header.push(Line::from(note.as_str()));
    }
    frame.render_widget(Paragraph::new(header).wrap(Wrap { trim: true }), chunks[0]);

    if picker.custom_mode {
        let content = vec![
            Line::from(Span::styled(
                "Custom version selector",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(picker.custom_buffer.as_str()),
        ];
        frame.render_widget(
            Paragraph::new(content)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::DarkGray)),
                )
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
    } else {
        let items = picker
            .choices
            .iter()
            .map(|choice| {
                let note = choice
                    .note
                    .as_deref()
                    .map(|note| format!("  {note}"))
                    .unwrap_or_default();
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<12}", choice.value),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(choice.label.clone()),
                    Span::styled(note, Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        state.select(Some(picker.selected));
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, chunks[1], &mut state);
    }

    let footer = if picker.custom_mode && picker.choices.is_empty() {
        "Enter apply   Ctrl+G cancel   Backspace delete"
    } else if picker.custom_mode {
        "Enter apply   Ctrl+G list   Backspace delete"
    } else {
        "Enter select   C custom   type a number to enter custom   Q cancel"
    };
    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

fn render_confirm_popup(frame: &mut Frame, area: Rect, confirm: &ConfirmState) {
    let popup = centered_rect(72, 9, area);
    frame.render_widget(Clear, popup);
    let content = vec![
        Line::from(Span::styled(
            confirm.title.as_str(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(confirm.message.as_str()),
        Line::from(""),
        Line::from("A apply   Q cancel"),
    ];
    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Confirm ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, popup);
}

fn render_action_loading_popup(frame: &mut Frame, area: Rect, task: &ActionTaskState) {
    let popup = centered_rect(64, 7, area);
    frame.render_widget(Clear, popup);
    let content = vec![
        Line::from(Span::styled(
            task.title.as_str(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Running inside the TUI..."),
    ];
    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Working ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, popup);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(height.min(area.height)),
            Constraint::Percentage(50),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(width.min(area.width)),
            Constraint::Percentage(50),
        ])
        .split(vertical[1]);
    horizontal[1]
}

impl VersionPickerState {
    fn new(
        target: FieldTarget,
        label: String,
        current: String,
        candidates: VersionCandidates,
    ) -> Self {
        let selected = candidates
            .candidates
            .iter()
            .position(|candidate| candidate.value == current)
            .unwrap_or(0);
        let custom_mode = candidates.candidates.is_empty();
        Self {
            target,
            label,
            source: candidates.source,
            note: candidates.note,
            choices: candidates.candidates,
            selected,
            custom_buffer: current,
            custom_mode,
        }
    }

    fn custom_only(target: FieldTarget, label: String, current: String) -> Self {
        Self {
            target,
            label,
            source: "custom input".to_string(),
            note: Some("enter an exact version or a major selector such as 24".to_string()),
            choices: Vec::new(),
            selected: 0,
            custom_buffer: current,
            custom_mode: true,
        }
    }
}

impl ActionOutput {
    fn ok(title: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            title: title.into(),
            lines,
            ok: true,
            mark_saved: false,
        }
    }

    fn saved(result: InitWriteResult) -> Self {
        let action = if result.overwritten {
            "Overwrote"
        } else {
            "Wrote"
        };
        Self {
            title: "Saved config".to_string(),
            lines: vec![format!("{action} {}", result.path.display())],
            ok: true,
            mark_saved: true,
        }
    }

    fn error(title: String, error: String) -> Self {
        Self {
            title,
            lines: vec!["Error".to_string(), String::new(), error],
            ok: false,
            mark_saved: false,
        }
    }
}

fn menu_entries() -> Vec<MenuEntry> {
    let mut entries = Vec::with_capacity(MENU_LEN);
    entries.push(MenuEntry::Policy);
    entries.extend(TOOL_NAMES.iter().copied().map(MenuEntry::Tool));
    entries.push(MenuEntry::Homebrew);
    entries.push(MenuEntry::Npm);
    entries.push(MenuEntry::Actions);
    entries.push(MenuEntry::Preview);
    entries
}

fn action_menu_index() -> usize {
    TOOL_NAMES.len() + 3
}

fn current_menu_entry(index: usize) -> MenuEntry {
    menu_entries()
        .get(index)
        .copied()
        .unwrap_or(MenuEntry::Policy)
}

fn menu_line(draft: &InitDraft, entry: MenuEntry) -> Line<'static> {
    match entry {
        MenuEntry::Policy => Line::from(vec![
            Span::styled("policy", Style::default().fg(Color::Yellow)),
            Span::raw(format!(
                "  {} / {}",
                draft.policy.channel, draft.policy.platform
            )),
        ]),
        MenuEntry::Tool(tool) => {
            let enabled = tool_enabled(draft, tool);
            let marker = if enabled { ENABLED_DOT } else { DISABLED_DOT };
            let color = if enabled {
                Color::Green
            } else {
                Color::DarkGray
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(tool.to_string(), Style::default().fg(color)),
                Span::raw(format!("  {}", tool_summary(draft, tool))),
            ])
        }
        MenuEntry::Homebrew => Line::from(vec![
            Span::styled("homebrew", Style::default().fg(Color::Yellow)),
            Span::raw(format!(
                "  {}",
                package_count(draft.cli.as_ref().map(|cli| &cli.packages))
            )),
        ]),
        MenuEntry::Npm => Line::from(vec![
            Span::styled("npm globals", Style::default().fg(Color::Yellow)),
            Span::raw(format!(
                "  {}",
                package_count(draft.npm_config.as_ref().map(|npm| &npm.global_packages))
            )),
        ]),
        MenuEntry::Actions => Line::from(vec![
            Span::styled("actions", Style::default().fg(Color::Green)),
            Span::raw("  save / check / sync"),
        ]),
        MenuEntry::Preview => Line::from(vec![Span::styled(
            "preview",
            Style::default().fg(Color::Magenta),
        )]),
    }
}

fn entry_title(entry: MenuEntry) -> &'static str {
    match entry {
        MenuEntry::Policy => "Policy",
        MenuEntry::Tool(tool) => tool,
        MenuEntry::Homebrew => "Homebrew packages",
        MenuEntry::Npm => "npm globals",
        MenuEntry::Actions => "Actions",
        MenuEntry::Preview => "Preview",
    }
}

fn entry_accent(entry: MenuEntry) -> Color {
    match entry {
        MenuEntry::Policy => Color::Cyan,
        MenuEntry::Tool("node" | "go" | "rust") => Color::Magenta,
        MenuEntry::Tool(_) => Color::Blue,
        MenuEntry::Homebrew => Color::Yellow,
        MenuEntry::Npm => Color::LightBlue,
        MenuEntry::Actions => Color::Green,
        MenuEntry::Preview => Color::Magenta,
    }
}

fn field_label_style(target: &FieldTarget) -> Style {
    let color = match target {
        FieldTarget::Action(ActionTarget::ApplySync) => Color::Red,
        FieldTarget::Action(ActionTarget::Finish | ActionTarget::SaveConfig) => Color::Green,
        FieldTarget::Action(_) => Color::Cyan,
        FieldTarget::ToolEnabled(_) => Color::Yellow,
        FieldTarget::NodeVersion | FieldTarget::GoVersion | FieldTarget::RustChannel => {
            Color::Magenta
        }
        FieldTarget::HomebrewPackages | FieldTarget::NpmGlobals => Color::Yellow,
        _ => Color::Cyan,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn field_value_style(target: &FieldTarget, value: &str) -> Style {
    let color = match target {
        FieldTarget::ToolEnabled(_) if truthy(value) => Color::Green,
        FieldTarget::ToolEnabled(_) => Color::DarkGray,
        FieldTarget::Action(ActionTarget::ApplySync) => Color::Red,
        FieldTarget::Action(ActionTarget::Finish | ActionTarget::SaveConfig) => Color::Green,
        FieldTarget::Action(_) => Color::White,
        FieldTarget::NodeVersion | FieldTarget::GoVersion | FieldTarget::RustChannel => {
            Color::Magenta
        }
        FieldTarget::SimpleManager(_) | FieldTarget::NodeManager | FieldTarget::GoManager => {
            Color::Cyan
        }
        FieldTarget::HomebrewPackages
        | FieldTarget::NpmGlobals
        | FieldTarget::NodePackageManagers => Color::Yellow,
        _ => Color::White,
    };
    Style::default().fg(color)
}

fn field_rows(draft: &InitDraft, entry: MenuEntry, options: &InitTuiOptions) -> Vec<FieldRow> {
    match entry {
        MenuEntry::Policy => vec![
            FieldRow {
                label: "channel".to_string(),
                value: draft.policy.channel.clone(),
                target: FieldTarget::PolicyChannel,
            },
            FieldRow {
                label: "platform".to_string(),
                value: draft.policy.platform.clone(),
                target: FieldTarget::PolicyPlatform,
            },
        ],
        MenuEntry::Tool(tool) => tool_field_rows(draft, tool),
        MenuEntry::Homebrew => vec![FieldRow {
            label: "packages".to_string(),
            value: list_value(draft.cli.as_ref().map(|cli| &cli.packages)),
            target: FieldTarget::HomebrewPackages,
        }],
        MenuEntry::Npm => vec![FieldRow {
            label: "global packages".to_string(),
            value: list_value(draft.npm_config.as_ref().map(|npm| &npm.global_packages)),
            target: FieldTarget::NpmGlobals,
        }],
        MenuEntry::Actions => action_field_rows(options),
        MenuEntry::Preview => Vec::new(),
    }
}

fn action_field_rows(options: &InitTuiOptions) -> Vec<FieldRow> {
    let output = if options.stdout {
        "show generated TOML".to_string()
    } else {
        options.output.display().to_string()
    };
    vec![
        FieldRow {
            label: "save config".to_string(),
            value: output,
            target: FieldTarget::Action(ActionTarget::SaveConfig),
        },
        FieldRow {
            label: "run check".to_string(),
            value: "doctor against current policy".to_string(),
            target: FieldTarget::Action(ActionTarget::RunCheck),
        },
        FieldRow {
            label: "preview sync".to_string(),
            value: "dry-run plan".to_string(),
            target: FieldTarget::Action(ActionTarget::PreviewSync),
        },
        FieldRow {
            label: "apply sync".to_string(),
            value: "install/configure now".to_string(),
            target: FieldTarget::Action(ActionTarget::ApplySync),
        },
        FieldRow {
            label: "finish".to_string(),
            value: if options.stdout {
                "print TOML and exit".to_string()
            } else {
                "save and exit".to_string()
            },
            target: FieldTarget::Action(ActionTarget::Finish),
        },
    ]
}

fn save_config_output(
    draft: &InitDraft,
    options: &InitTuiOptions,
    force: bool,
) -> Result<ActionOutput> {
    let document = render_init_document(draft);
    if options.stdout {
        let mut lines = vec![
            "Generated TOML".to_string(),
            "Finish will print this document after leaving the TUI.".to_string(),
            String::new(),
        ];
        lines.extend(document.content.lines().map(ToString::to_string));
        return Ok(ActionOutput::ok("Generated TOML", lines));
    }

    let result = write_init_document(&options.output, force, &document)?;
    Ok(ActionOutput::saved(result))
}

fn config_from_draft(draft: &InitDraft) -> Result<DevkitConfig> {
    let content = render_init_document(draft).content;
    let config =
        toml::from_str::<DevkitConfig>(&content).context("generated TOML could not be parsed")?;
    Ok(config.effective_for_current_platform())
}

fn doctor_output(report: DoctorReport, options: &InitTuiOptions) -> ActionOutput {
    let mut lines = vec![
        "Doctor report".to_string(),
        format!(
            "Policy source: {}",
            if options.stdout {
                "current TUI draft".to_string()
            } else {
                options.output.display().to_string()
            }
        ),
        String::new(),
        "Tools".to_string(),
    ];
    for tool in &report.tools {
        lines.push(format!(
            "- {:<9} {:<8} current {:<12} required {:<12} {}",
            tool.name,
            status_label(&tool.status),
            tool.current.as_deref().unwrap_or("-"),
            tool.required.as_deref().unwrap_or("-"),
            tool.path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        if let Some(note) = &tool.note {
            lines.push(format!("  note: {note}"));
        }
    }

    if report.issues.is_empty() {
        lines.push(String::new());
        lines.push("Issues: none".to_string());
    } else {
        lines.push(String::new());
        lines.push("Issues".to_string());
        for issue in &report.issues {
            lines.push(format!("- {}", issue.message));
            if let Some(path) = &issue.path {
                lines.push(format!("  path: {}", path.display()));
            }
            if let Some(fix) = &issue.fix {
                lines.push(format!("  fix: {fix}"));
            }
        }
    }

    let ok = report
        .tools
        .iter()
        .all(|tool| matches!(tool.status, Status::Ok) || tool.required.is_none());
    ActionOutput {
        title: "Check environment".to_string(),
        lines,
        ok,
        mark_saved: false,
    }
}

fn sync_plan_output(plan: SyncPlan) -> ActionOutput {
    let mut lines = vec![format!(
        "Sync plan{}",
        if plan.dry_run { " (dry-run)" } else { "" }
    )];
    if let Some(channel) = &plan.policy_channel {
        lines.push(format!("Target channel: {channel}"));
    }
    if let Some(platform) = &plan.platform {
        lines.push(format!("Target platform: {platform}"));
    }
    if plan.auto_fix {
        lines.push("Policy auto-fix: enabled".to_string());
    }
    lines.push(String::new());

    let ready_steps = plan
        .graph
        .ready
        .iter()
        .filter_map(|id| plan.steps.iter().find(|step| step.id == *id))
        .collect::<Vec<_>>();
    if !ready_steps.is_empty() {
        lines.push("Ready".to_string());
        for step in ready_steps {
            push_sync_step_lines(&mut lines, step);
        }
        lines.push(String::new());
    }

    if !plan.graph.blocked.is_empty() {
        lines.push("Blocked".to_string());
        for blocked in &plan.graph.blocked {
            if let Some(step) = plan.steps.iter().find(|step| step.id == blocked.id) {
                push_sync_step_lines(&mut lines, step);
            }
        }
        lines.push(String::new());
    }

    let verify_steps = plan
        .steps
        .iter()
        .filter(|step| matches!(step.kind, SyncStepKind::Verify))
        .collect::<Vec<_>>();
    if !verify_steps.is_empty() {
        lines.push("Verify".to_string());
        for step in verify_steps {
            push_sync_step_lines(&mut lines, step);
        }
    }

    ActionOutput::ok("Preview sync", lines)
}

fn push_sync_step_lines(lines: &mut Vec<String>, step: &crate::sync::SyncStep) {
    lines.push(format!(
        "- [{}] {} - {}",
        kind_label(&step.kind),
        step.target,
        step.reason
    ));
    if !step.blocked_by.is_empty() {
        lines.push(format!("   blocked by: {}", step.blocked_by.join(", ")));
    }
    if let Some(command) = &step.command {
        let label = if step.manual {
            "instruction"
        } else {
            "command"
        };
        lines.push(format!("   {label}: {command}"));
    }
    if let Some(file) = &step.file {
        lines.push(format!("   file: {}", file.display()));
    }
    if let Some(snippet) = &step.snippet {
        lines.push(format!(
            "   snippet: {}",
            snippet.replace('\n', "\n            ")
        ));
    }
    if step.requires_sudo {
        lines.push("   requires sudo: yes".to_string());
    }
}

fn sync_execution_output(execution: SyncExecution) -> ActionOutput {
    let mut lines = vec!["Sync execution".to_string()];
    if let Some(channel) = &execution.policy_channel {
        lines.push(format!("Target channel: {channel}"));
    }
    if let Some(platform) = &execution.platform {
        lines.push(format!("Target platform: {platform}"));
    }
    if execution.auto_fix {
        lines.push("Policy auto-fix: enabled".to_string());
    }
    lines.push(String::new());

    for step in &execution.steps {
        lines.push(format!(
            "- [{}] {}: {}",
            execution_status_label(&step.status),
            step.target,
            step.detail
        ));
        if !step.blocked_by.is_empty() {
            lines.push(format!("  blocked by: {}", step.blocked_by.join(", ")));
        }
        if let Some(command) = &step.command {
            let label = if step.manual {
                "instruction"
            } else {
                "command"
            };
            lines.push(format!("  {label}: {command}"));
        }
        if let Some(file) = &step.file {
            lines.push(format!("  file: {}", file.display()));
        }
    }

    lines.push(String::new());
    if execution.succeeded {
        lines.push("Result: environment matches policy".to_string());
    } else {
        lines.push("Result: sync stopped before reaching the configured policy".to_string());
    }

    ActionOutput {
        title: "Apply sync".to_string(),
        lines,
        ok: execution.succeeded,
        mark_saved: false,
    }
}

fn status_label(status: &Status) -> &'static str {
    match status {
        Status::Ok => "ok",
        Status::Missing => "missing",
        Status::Mismatch => "mismatch",
        Status::Unknown => "unknown",
    }
}

fn kind_label(kind: &SyncStepKind) -> &'static str {
    match kind {
        SyncStepKind::Install => "install",
        SyncStepKind::Align => "align",
        SyncStepKind::Configure => "configure",
        SyncStepKind::Cleanup => "cleanup",
        SyncStepKind::Verify => "verify",
        SyncStepKind::Info => "info",
    }
}

fn execution_status_label(status: &SyncStepExecutionStatus) -> &'static str {
    match status {
        SyncStepExecutionStatus::Applied => "applied",
        SyncStepExecutionStatus::Unchanged => "unchanged",
        SyncStepExecutionStatus::Skipped => "skipped",
        SyncStepExecutionStatus::Failed => "failed",
        SyncStepExecutionStatus::Verified => "verified",
    }
}

fn tool_field_rows(draft: &InitDraft, tool: &'static str) -> Vec<FieldRow> {
    let mut rows = vec![FieldRow {
        label: "enabled".to_string(),
        value: if tool_enabled(draft, tool) {
            "yes".to_string()
        } else {
            "no".to_string()
        },
        target: FieldTarget::ToolEnabled(tool),
    }];
    if !tool_enabled(draft, tool) {
        return rows;
    }

    match tool {
        "node" => {
            if let Some(node) = &draft.node {
                rows.push(FieldRow {
                    label: "version".to_string(),
                    value: node.version.clone(),
                    target: FieldTarget::NodeVersion,
                });
                rows.push(FieldRow {
                    label: "manager".to_string(),
                    value: node.manager.clone(),
                    target: FieldTarget::NodeManager,
                });
                rows.push(FieldRow {
                    label: "package managers".to_string(),
                    value: list_value(Some(&node.package_managers)),
                    target: FieldTarget::NodePackageManagers,
                });
            }
        }
        "go" => {
            if let Some(go) = &draft.go {
                rows.push(FieldRow {
                    label: "version".to_string(),
                    value: go.version.clone(),
                    target: FieldTarget::GoVersion,
                });
                rows.push(FieldRow {
                    label: "manager".to_string(),
                    value: go.manager.clone(),
                    target: FieldTarget::GoManager,
                });
                rows.push(FieldRow {
                    label: "source".to_string(),
                    value: go.source.clone(),
                    target: FieldTarget::GoSource,
                });
                rows.push(FieldRow {
                    label: "install dir".to_string(),
                    value: go.install_dir.clone().unwrap_or_default(),
                    target: FieldTarget::GoInstallDir,
                });
            }
        }
        "rust" => {
            if let Some(rust) = &draft.rust {
                rows.push(FieldRow {
                    label: "channel".to_string(),
                    value: rust.channel.clone(),
                    target: FieldTarget::RustChannel,
                });
            }
        }
        simple => {
            if let Some(tool) = simple_tool(draft, simple) {
                rows.push(FieldRow {
                    label: "manager".to_string(),
                    value: tool.manager.clone(),
                    target: FieldTarget::SimpleManager(simple),
                });
            }
        }
    }

    rows
}

fn apply_field_edit(draft: &mut InitDraft, target: &FieldTarget, value: &str) {
    match target {
        FieldTarget::PolicyChannel => draft.policy.channel = value.to_string(),
        FieldTarget::PolicyPlatform => draft.policy.platform = value.to_string(),
        FieldTarget::ToolEnabled(tool) => set_tool_enabled(draft, tool, truthy(value)),
        FieldTarget::SimpleManager(tool) => {
            if let Some(simple) = simple_tool_mut(draft, tool) {
                simple.manager = value.to_string();
            }
        }
        FieldTarget::NodeVersion => {
            if let Some(node) = &mut draft.node {
                node.version = value.to_string();
            }
        }
        FieldTarget::NodeManager => {
            if let Some(node) = &mut draft.node {
                node.manager = value.to_string();
            }
        }
        FieldTarget::NodePackageManagers => {
            if let Some(node) = &mut draft.node {
                node.package_managers = parse_list(value);
            }
        }
        FieldTarget::GoVersion => {
            if let Some(go) = &mut draft.go {
                go.version = value.to_string();
            }
        }
        FieldTarget::GoManager => {
            if let Some(go) = &mut draft.go {
                go.manager = value.to_string();
            }
        }
        FieldTarget::GoSource => {
            if let Some(go) = &mut draft.go {
                go.source = value.to_string();
                if go.source != "official" {
                    go.install_dir = None;
                }
            }
        }
        FieldTarget::GoInstallDir => {
            if let Some(go) = &mut draft.go {
                go.install_dir = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
        }
        FieldTarget::RustChannel => {
            if let Some(rust) = &mut draft.rust {
                rust.channel = value.to_string();
            }
        }
        FieldTarget::HomebrewPackages => {
            let packages = parse_list(value);
            if packages.is_empty() {
                draft.cli = None;
                draft.homebrew = None;
            } else {
                draft.cli = Some(CliDraft {
                    manager: "brew".to_string(),
                    packages: packages.clone(),
                });
                draft.homebrew = Some(HomebrewDraft { packages });
            }
        }
        FieldTarget::NpmGlobals => {
            let global_packages = parse_list(value);
            draft.npm_config = if global_packages.is_empty() {
                None
            } else {
                Some(NpmDraft { global_packages })
            };
        }
        FieldTarget::Action(_) => {}
    }
}

fn tool_enabled(draft: &InitDraft, tool: &str) -> bool {
    match tool {
        "fnm" => draft.fnm.is_some(),
        "nvm" => draft.nvm.is_some(),
        "node" => draft.node.is_some(),
        "npm" => draft.npm.is_some(),
        "pnpm" => draft.pnpm.is_some(),
        "yarn" => draft.yarn.is_some(),
        "bun" => draft.bun.is_some(),
        "deno" => draft.deno.is_some(),
        "go" => draft.go.is_some(),
        "rust" => draft.rust.is_some(),
        "uv" => draft.uv.is_some(),
        "python" => draft.python.is_some(),
        "poetry" => draft.poetry.is_some(),
        "ruby" => draft.ruby.is_some(),
        "wrangler" => draft.wrangler.is_some(),
        _ => false,
    }
}

fn toggle_tool(draft: &mut InitDraft, tool: &'static str) {
    let enabled = !tool_enabled(draft, tool);
    set_tool_enabled(draft, tool, enabled);
}

fn set_tool_enabled(draft: &mut InitDraft, tool: &str, enabled: bool) {
    match tool {
        "fnm" => set_simple_tool(&mut draft.fnm, enabled, default_manager("fnm")),
        "nvm" => set_simple_tool(&mut draft.nvm, enabled, "standalone"),
        "node" => {
            if enabled {
                draft.node.get_or_insert_with(default_node_draft);
            } else {
                draft.node = None;
            }
        }
        "npm" => set_simple_tool(&mut draft.npm, enabled, "npm"),
        "pnpm" => set_simple_tool(&mut draft.pnpm, enabled, "corepack"),
        "yarn" => set_simple_tool(&mut draft.yarn, enabled, "corepack"),
        "bun" => set_simple_tool(&mut draft.bun, enabled, default_manager("bun")),
        "deno" => set_simple_tool(&mut draft.deno, enabled, default_manager("deno")),
        "go" => {
            if enabled {
                draft.go.get_or_insert_with(default_go_draft);
            } else {
                draft.go = None;
            }
        }
        "rust" => {
            if enabled {
                draft.rust.get_or_insert_with(default_rust_draft);
            } else {
                draft.rust = None;
            }
        }
        "uv" => set_simple_tool(&mut draft.uv, enabled, "standalone"),
        "python" => set_simple_tool(&mut draft.python, enabled, default_manager("python")),
        "poetry" => set_simple_tool(&mut draft.poetry, enabled, default_manager("poetry")),
        "ruby" => set_simple_tool(&mut draft.ruby, enabled, default_manager("ruby")),
        "wrangler" => set_simple_tool(&mut draft.wrangler, enabled, "npm"),
        _ => {}
    }
}

fn set_simple_tool(tool: &mut Option<SimpleToolDraft>, enabled: bool, default_manager: &str) {
    if enabled {
        tool.get_or_insert_with(|| SimpleToolDraft {
            manager: default_manager.to_string(),
        });
    } else {
        *tool = None;
    }
}

fn simple_tool<'a>(draft: &'a InitDraft, tool: &str) -> Option<&'a SimpleToolDraft> {
    match tool {
        "fnm" => draft.fnm.as_ref(),
        "nvm" => draft.nvm.as_ref(),
        "npm" => draft.npm.as_ref(),
        "pnpm" => draft.pnpm.as_ref(),
        "yarn" => draft.yarn.as_ref(),
        "bun" => draft.bun.as_ref(),
        "deno" => draft.deno.as_ref(),
        "uv" => draft.uv.as_ref(),
        "python" => draft.python.as_ref(),
        "poetry" => draft.poetry.as_ref(),
        "ruby" => draft.ruby.as_ref(),
        "wrangler" => draft.wrangler.as_ref(),
        _ => None,
    }
}

fn simple_tool_mut<'a>(draft: &'a mut InitDraft, tool: &str) -> Option<&'a mut SimpleToolDraft> {
    match tool {
        "fnm" => draft.fnm.as_mut(),
        "nvm" => draft.nvm.as_mut(),
        "npm" => draft.npm.as_mut(),
        "pnpm" => draft.pnpm.as_mut(),
        "yarn" => draft.yarn.as_mut(),
        "bun" => draft.bun.as_mut(),
        "deno" => draft.deno.as_mut(),
        "uv" => draft.uv.as_mut(),
        "python" => draft.python.as_mut(),
        "poetry" => draft.poetry.as_mut(),
        "ruby" => draft.ruby.as_mut(),
        "wrangler" => draft.wrangler.as_mut(),
        _ => None,
    }
}

fn default_node_draft() -> NodeDraft {
    NodeDraft {
        version: "stable".to_string(),
        manager: "fnm".to_string(),
        package_managers: vec![
            "npm".to_string(),
            "pnpm".to_string(),
            "yarn".to_string(),
            "bun".to_string(),
        ],
    }
}

fn default_go_draft() -> GoDraft {
    let platform = OperatingSystem::current();
    let source = platform.default_go_source();
    GoDraft {
        version: "stable".to_string(),
        manager: source.to_string(),
        source: source.to_string(),
        install_dir: (source == "official").then(|| "~/.local/opt/go/current".to_string()),
    }
}

fn default_manager(tool: &str) -> &'static str {
    OperatingSystem::current()
        .default_manager_for(tool)
        .unwrap_or("manual")
}

fn default_rust_draft() -> RustDraft {
    RustDraft {
        channel: "stable".to_string(),
    }
}

fn parse_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("none"))
        .map(ToString::to_string)
        .collect()
}

fn list_value(items: Option<&Vec<String>>) -> String {
    items
        .filter(|items| !items.is_empty())
        .map(|items| items.join(", "))
        .unwrap_or_default()
}

fn package_count(items: Option<&Vec<String>>) -> String {
    match items.map(Vec::len).unwrap_or_default() {
        0 => "none".to_string(),
        1 => "1 item".to_string(),
        count => format!("{count} items"),
    }
}

fn preview_line_count(draft: &InitDraft) -> u16 {
    let count = render_init_document(draft).content.lines().count();
    count.min(u16::MAX as usize) as u16
}

fn tool_summary(draft: &InitDraft, tool: &str) -> String {
    match tool {
        "node" => draft
            .node
            .as_ref()
            .map(|node| format!("{} via {}", node.version, node.manager))
            .unwrap_or_default(),
        "go" => draft
            .go
            .as_ref()
            .map(|go| format!("{} via {}", go.version, go.manager))
            .unwrap_or_default(),
        "rust" => draft
            .rust
            .as_ref()
            .map(|rust| rust.channel.clone())
            .unwrap_or_default(),
        simple => simple_tool(draft, simple)
            .map(|tool| format!("via {}", tool.manager))
            .unwrap_or_default(),
    }
}

fn enabled_label(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "disabled" }
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "on" | "enabled"
    )
}

fn version_target_tool(target: &FieldTarget) -> Option<&'static str> {
    match target {
        FieldTarget::NodeVersion => Some("node"),
        FieldTarget::GoVersion => Some("go"),
        FieldTarget::RustChannel => Some("rust"),
        _ => None,
    }
}

fn is_version_character(character: char) -> bool {
    character.is_ascii_digit() || matches!(character, '.' | 'x' | 'X' | 'v' | 'V' | '-')
}

fn move_index(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let index = index % len;
    let steps = delta.unsigned_abs() % len;
    if steps == 0 {
        return index;
    }

    if delta.is_negative() {
        if steps <= index {
            index - steps
        } else {
            len - (steps - index)
        }
    } else if index < len - steps {
        index + steps
    } else {
        steps - (len - index)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::init::{InitDraft, PolicyDraft, render_init_document};
    use crate::latest::{VersionCandidate, VersionCandidates};
    use crate::sync::{SyncBlockedStep, SyncPlan, SyncPlanGraph, SyncStep, SyncStepKind};

    use super::{
        ActionOutput, AppExit, FieldTarget, Focus, InitTuiApp, InitTuiOptions, VersionPickerState,
        action_menu_index, apply_field_edit, default_node_draft, menu_entries, move_index,
        parse_list, set_tool_enabled, sync_plan_output, tool_enabled,
    };

    #[test]
    fn toggles_tools_with_defaults() {
        let mut draft = empty_draft();

        set_tool_enabled(&mut draft, "node", true);
        set_tool_enabled(&mut draft, "nvm", true);
        set_tool_enabled(&mut draft, "bun", true);
        set_tool_enabled(&mut draft, "go", true);

        assert!(tool_enabled(&draft, "node"));
        assert!(tool_enabled(&draft, "nvm"));
        assert_eq!(draft.node.as_ref().unwrap().manager, "fnm");
        assert_eq!(draft.nvm.as_ref().unwrap().manager, "standalone");
        assert_eq!(draft.bun.as_ref().unwrap().manager, "brew");
        assert_eq!(draft.go.as_ref().unwrap().source, "brew");

        let document = render_init_document(&draft);
        assert!(document.content.contains("[tools.nvm]"));
        assert!(document.content.contains("manager = \"standalone\""));

        set_tool_enabled(&mut draft, "node", false);
        assert!(!tool_enabled(&draft, "node"));
    }

    #[test]
    fn sync_preview_groups_ready_blocked_and_verify_steps() {
        let plan = SyncPlan {
            dry_run: true,
            policy_channel: Some("stable".to_string()),
            platform: Some("macos-arm64".to_string()),
            auto_fix: false,
            graph: SyncPlanGraph {
                ready: vec!["tool:nvm".to_string()],
                blocked: vec![SyncBlockedStep {
                    id: "tool:node".to_string(),
                    target: "node".to_string(),
                    blocked_by: vec!["tool:nvm".to_string()],
                }],
            },
            steps: vec![
                SyncStep {
                    id: "tool:nvm".to_string(),
                    kind: SyncStepKind::Install,
                    target: "nvm".to_string(),
                    reason: "install nvm required by the configured Node workflow".to_string(),
                    blocked_by: Vec::new(),
                    command: Some("install nvm".to_string()),
                    file: None,
                    snippet: None,
                    manual: false,
                    requires_sudo: false,
                },
                SyncStep {
                    id: "tool:node".to_string(),
                    kind: SyncStepKind::Install,
                    target: "node".to_string(),
                    reason: "install node with the configured manager".to_string(),
                    blocked_by: vec!["tool:nvm".to_string()],
                    command: Some("nvm install 24".to_string()),
                    file: None,
                    snippet: None,
                    manual: false,
                    requires_sudo: false,
                },
                SyncStep {
                    id: "verify:doctor".to_string(),
                    kind: SyncStepKind::Verify,
                    target: "environment".to_string(),
                    reason: "re-run doctor after applying the planned steps".to_string(),
                    blocked_by: Vec::new(),
                    command: Some("devkit doctor --config devkit.toml".to_string()),
                    file: None,
                    snippet: None,
                    manual: false,
                    requires_sudo: false,
                },
            ],
        };

        let output = sync_plan_output(plan);
        let text = output.lines.join("\n");

        assert!(text.contains("Ready"));
        assert!(
            text.contains("- [install] nvm - install nvm required by the configured Node workflow")
        );
        assert!(text.contains("Blocked"));
        assert!(text.contains("- [install] node - install node with the configured manager"));
        assert!(text.contains("blocked by: tool:nvm"));
        assert!(text.contains("Verify"));
        assert!(
            text.contains(
                "- [verify] environment - re-run doctor after applying the planned steps"
            )
        );
    }

    #[test]
    fn edits_fields_in_draft() {
        let mut draft = empty_draft();
        draft.node = Some(default_node_draft());

        apply_field_edit(&mut draft, &FieldTarget::PolicyChannel, "beta");
        apply_field_edit(&mut draft, &FieldTarget::NodeVersion, "24.x");
        apply_field_edit(
            &mut draft,
            &FieldTarget::NodePackageManagers,
            "npm, pnpm, bun",
        );
        apply_field_edit(&mut draft, &FieldTarget::HomebrewPackages, "gh, tmux");

        assert_eq!(draft.policy.channel, "beta");
        assert_eq!(draft.node.as_ref().unwrap().version, "24.x");
        assert_eq!(
            draft.node.as_ref().unwrap().package_managers,
            vec!["npm".to_string(), "pnpm".to_string(), "bun".to_string()]
        );
        assert_eq!(
            draft.cli.as_ref().unwrap().packages,
            vec!["gh".to_string(), "tmux".to_string()]
        );
    }

    #[test]
    fn parses_comma_lists_and_none() {
        assert_eq!(
            parse_list("npm, pnpm, bun"),
            vec!["npm".to_string(), "pnpm".to_string(), "bun".to_string()]
        );
        assert!(parse_list("none").is_empty());
    }

    #[test]
    fn selection_movement_wraps_at_edges() {
        assert_eq!(move_index(0, 5, -1), 4);
        assert_eq!(move_index(4, 5, 1), 0);
        assert_eq!(move_index(1, 5, -8), 3);
        assert_eq!(move_index(3, 5, 8), 1);
    }

    #[test]
    fn up_from_first_menu_wraps_to_last_entry() {
        let mut app = InitTuiApp::new(empty_draft());
        app.focus = Focus::Menu;
        app.menu_index = 0;

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));

        assert_eq!(app.menu_index, menu_entries().len() - 1);
    }

    #[test]
    fn left_and_right_choose_expected_pane() {
        let mut app = InitTuiApp::new(empty_draft());

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Fields);

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Fields);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Menu);
    }

    #[test]
    fn full_preview_returns_to_previous_editor_selection() {
        let mut app = InitTuiApp::new(empty_draft());
        app.menu_index = 8;
        app.field_index = 2;
        app.focus = Focus::Fields;

        app.open_preview();
        assert!(app.preview_expanded);
        app.close_preview();

        assert!(!app.preview_expanded);
        assert_eq!(app.menu_index, 8);
        assert_eq!(app.field_index, 2);
        assert_eq!(app.focus, Focus::Fields);
    }

    #[test]
    fn actions_output_can_expand_and_return_to_actions() {
        let mut app = InitTuiApp::new(empty_draft());
        app.menu_index = action_menu_index();
        app.field_index = 1;
        app.focus = Focus::Fields;
        app.action_output = Some(ActionOutput::ok(
            "Check environment",
            (0..30).map(|index| format!("line {index}")).collect(),
        ));

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        assert!(app.action_expanded);
        assert!(!app.preview_expanded);

        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert!(app.action_scroll > 0);

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        assert!(!app.action_expanded);
        assert_eq!(app.menu_index, action_menu_index());
        assert_eq!(app.field_index, 1);
    }

    #[test]
    fn version_picker_accepts_custom_major_selector() {
        let mut app = InitTuiApp::new(empty_draft());
        app.draft.node = Some(default_node_draft());
        app.version_picker = Some(VersionPickerState::custom_only(
            FieldTarget::NodeVersion,
            "version".to_string(),
            String::new(),
        ));

        app.handle_version_picker_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        app.handle_version_picker_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
        app.handle_version_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.draft.node.as_ref().unwrap().version, "24");
        assert!(app.version_picker.is_none());
    }

    #[test]
    fn version_picker_keeps_open_on_escape_prefix() {
        let mut app = InitTuiApp::new(empty_draft());
        app.version_picker = Some(VersionPickerState::new(
            FieldTarget::NodeVersion,
            "version".to_string(),
            "24.x".to_string(),
            VersionCandidates {
                candidates: vec![
                    VersionCandidate {
                        label: "Node 24.x".to_string(),
                        value: "24.x".to_string(),
                        note: None,
                    },
                    VersionCandidate {
                        label: "Node 25.x".to_string(),
                        value: "25.x".to_string(),
                        note: None,
                    },
                ],
                source: "test".to_string(),
                note: None,
            },
        ));

        app.handle_version_picker_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.version_picker.is_some());

        app.handle_version_picker_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.version_picker.as_ref().unwrap().selected, 1);
    }

    #[test]
    fn escape_does_not_exit_main_or_preview_subscreen() {
        let mut app = InitTuiApp::new(empty_draft());

        let exit = app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(exit.is_none());

        app.open_preview();
        let exit = app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(exit.is_none());
        assert!(app.preview_expanded);
    }

    #[test]
    fn escape_does_not_close_edit_popup() {
        let mut app = InitTuiApp::new(empty_draft());
        app.focus = Focus::Fields;
        app.start_edit_or_focus_fields();

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.edit.is_some());
        assert!(app.status.contains("Esc is ignored"));
    }

    #[test]
    fn ctrl_g_cancels_edit_popup_without_applying_buffer() {
        let mut app = InitTuiApp::new(empty_draft());
        app.focus = Focus::Fields;
        app.start_edit_or_focus_fields();

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));

        assert!(app.edit.is_none());
        assert_eq!(app.draft.policy.channel, "stable");
        assert_eq!(app.status, "Edit cancelled");
    }

    #[test]
    fn finish_action_saves_and_exits_inside_tui() {
        let path = unique_temp_path("devkit-tui-finish");
        let mut app = InitTuiApp::with_options(
            empty_draft(),
            InitTuiOptions {
                output: path.clone(),
                force: false,
                stdout: false,
            },
        );
        app.menu_index = action_menu_index();
        app.focus = Focus::Fields;
        app.field_index = 4;

        let exit = app.start_edit_or_focus_fields();

        assert!(matches!(exit, Some(AppExit::Handled)));
        assert!(fs::read_to_string(&path).unwrap().contains("[policy]"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn stdout_finish_returns_to_cli_print_path() {
        let path = unique_temp_path("devkit-tui-stdout");
        let mut app = InitTuiApp::with_options(
            empty_draft(),
            InitTuiOptions {
                output: path.clone(),
                force: false,
                stdout: true,
            },
        );
        app.menu_index = action_menu_index();
        app.focus = Focus::Fields;
        app.field_index = 4;

        let exit = app.start_edit_or_focus_fields();

        assert!(matches!(exit, Some(AppExit::Continue)));
        assert!(!path.exists());
    }

    fn empty_draft() -> InitDraft {
        InitDraft {
            policy: PolicyDraft {
                channel: "stable".to_string(),
                platform: "macos-arm64".to_string(),
            },
            fnm: None,
            nvm: None,
            node: None,
            npm: None,
            pnpm: None,
            yarn: None,
            bun: None,
            deno: None,
            go: None,
            rust: None,
            uv: None,
            python: None,
            poetry: None,
            ruby: None,
            wrangler: None,
            cli: None,
            homebrew: None,
            npm_config: None,
        }
    }

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{timestamp}.toml"))
    }
}
