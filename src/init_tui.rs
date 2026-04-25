use std::{
    io,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
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
    init::{
        CliDraft, GoDraft, HomebrewDraft, InitDraft, NodeDraft, NpmDraft, RustDraft,
        SimpleToolDraft, render_init_document,
    },
    latest::{VersionCandidate, VersionCandidates, lookup_version_candidates},
};

const TOOL_NAMES: &[&str] = &[
    "fnm", "node", "npm", "pnpm", "yarn", "bun", "deno", "go", "rust", "uv", "python", "poetry",
    "ruby", "wrangler",
];

const MENU_LEN: usize = TOOL_NAMES.len() + 4;

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
    Preview,
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

struct InitTuiApp {
    draft: InitDraft,
    menu_index: usize,
    field_index: usize,
    focus: Focus,
    edit: Option<EditState>,
    version_fetch: Option<VersionFetchState>,
    version_picker: Option<VersionPickerState>,
    status: String,
    preview_scroll: u16,
    preview_expanded: bool,
}

enum AppExit {
    Save,
    Cancel,
}

pub fn customize_init_draft_tui(draft: &mut InitDraft) -> Result<bool> {
    enable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_tui(&mut terminal, draft);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    draft: &mut InitDraft,
) -> Result<bool> {
    let mut app = InitTuiApp::new(draft.clone());
    let mut last_tick = Instant::now();

    loop {
        app.poll_version_fetch();
        terminal.draw(|frame| app.render(frame))?;

        let timeout = Duration::from_millis(250)
            .checked_sub(last_tick.elapsed())
            .unwrap_or_default();
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && let Some(exit) = app.handle_key(key)
        {
            match exit {
                AppExit::Save => {
                    *draft = app.draft;
                    return Ok(true);
                }
                AppExit::Cancel => return Ok(false),
            }
        }

        if last_tick.elapsed() >= Duration::from_millis(250) {
            last_tick = Instant::now();
        }
    }
}

impl InitTuiApp {
    fn new(draft: InitDraft) -> Self {
        Self {
            draft,
            menu_index: 0,
            field_index: 0,
            focus: Focus::Menu,
            edit: None,
            version_fetch: None,
            version_picker: None,
            status: "Tip: left sections, center fields, right live TOML preview".to_string(),
            preview_scroll: 0,
            preview_expanded: false,
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
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let enabled = TOOL_NAMES
            .iter()
            .filter(|tool| tool_enabled(&self.draft, tool))
            .count();
        let content = vec![
            Line::from(vec![
                Span::styled(
                    "devkit init",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  visual policy builder"),
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

        if area.width >= 112 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(30),
                    Constraint::Length(44),
                    Constraint::Min(36),
                ])
                .split(area);

            self.render_menu(frame, chunks[0]);
            self.render_fields(frame, chunks[1]);
            self.render_preview(frame, chunks[2], false);
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
            self.render_preview(frame, rows[1], false);
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
                    .border_style(Style::default().fg(border)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn render_fields(&mut self, frame: &mut Frame, area: Rect) {
        let entry = current_menu_entry(self.menu_index);
        let fields = field_rows(&self.draft, entry);
        if self.field_index >= fields.len() {
            self.field_index = fields.len().saturating_sub(1);
        }

        let border = if self.focus == Focus::Fields {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .title(format!(" {} ", entry_title(entry)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border));

        if fields.is_empty() {
            let text = match entry {
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
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(field.value.clone()),
                ]))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        state.select(Some(self.field_index));
        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("  ");
        frame.render_stateful_widget(list, area, &mut state);
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

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let keys = if self.preview_expanded {
            Line::from(vec![
                Span::styled("Up/Dn", Style::default().fg(Color::Cyan)),
                Span::raw(" scroll  "),
                Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
                Span::raw(" page  "),
                Span::styled("P/Tab", Style::default().fg(Color::Cyan)),
                Span::raw(" back  "),
                Span::styled("S", Style::default().fg(Color::Green)),
                Span::raw(" save  "),
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
                Span::raw(" preview  "),
                Span::styled("S", Style::default().fg(Color::Green)),
                Span::raw(" save  "),
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
            return Some(AppExit::Cancel);
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

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(AppExit::Cancel),
            KeyCode::Char('s') | KeyCode::Char('S') => Some(AppExit::Save),
            KeyCode::Tab | KeyCode::Right | KeyCode::Left => {
                self.toggle_focus();
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
                self.scroll_preview(-8);
                None
            }
            KeyCode::PageDown => {
                self.scroll_preview(8);
                None
            }
            KeyCode::Char(' ') => {
                self.toggle_current_tool_or_field();
                None
            }
            KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('E') => {
                self.start_edit_or_focus_fields();
                None
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.open_preview();
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
            KeyCode::Char('q') | KeyCode::Esc => Some(AppExit::Cancel),
            KeyCode::Char('s') | KeyCode::Char('S') => Some(AppExit::Save),
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
                    if picker.choices.is_empty() {
                        self.status =
                            "Enter a version selector, or press Ctrl-C to cancel init".to_string();
                        self.version_picker = Some(picker);
                    } else {
                        picker.custom_mode = false;
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
                self.status = "Edit cancelled".to_string();
            }
            KeyCode::Enter => {
                apply_field_edit(&mut self.draft, &edit.target, edit.buffer.trim());
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
                let fields = field_rows(&self.draft, current_menu_entry(self.menu_index));
                if !fields.is_empty() {
                    self.field_index = move_index(self.field_index, fields.len(), delta);
                }
            }
        }
    }

    fn toggle_current_tool_or_field(&mut self) {
        match self.focus {
            Focus::Menu => {
                if let MenuEntry::Tool(tool) = current_menu_entry(self.menu_index) {
                    toggle_tool(&mut self.draft, tool);
                    self.status =
                        format!("{tool} {}", enabled_label(tool_enabled(&self.draft, tool)));
                }
            }
            Focus::Fields => {
                if let Some(target) = self.current_field_target()
                    && let FieldTarget::ToolEnabled(tool) = target
                {
                    toggle_tool(&mut self.draft, tool);
                    self.status =
                        format!("{tool} {}", enabled_label(tool_enabled(&self.draft, tool)));
                }
            }
        }
    }

    fn start_edit_or_focus_fields(&mut self) {
        if self.focus == Focus::Menu {
            if matches!(current_menu_entry(self.menu_index), MenuEntry::Preview) {
                self.open_preview();
            } else {
                self.focus = Focus::Fields;
            }
            return;
        }

        let Some(field) = self.current_field() else {
            return;
        };
        if let FieldTarget::ToolEnabled(tool) = field.target {
            toggle_tool(&mut self.draft, tool);
            self.status = format!("{tool} {}", enabled_label(tool_enabled(&self.draft, tool)));
            return;
        }

        if version_target_tool(&field.target).is_some() {
            self.start_version_fetch(field);
            return;
        }

        self.edit = Some(EditState {
            target: field.target,
            label: field.label,
            buffer: field.value,
        });
    }

    fn open_preview(&mut self) {
        self.preview_expanded = true;
        self.status = "Full preview: arrows scroll, PgUp/PgDn page, P returns".to_string();
    }

    fn close_preview(&mut self) {
        self.preview_expanded = false;
        self.status = "Returned to editor. P opens full preview again.".to_string();
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

    fn current_field(&self) -> Option<FieldRow> {
        field_rows(&self.draft, current_menu_entry(self.menu_index))
            .get(self.field_index)
            .cloned()
    }

    fn current_field_target(&self) -> Option<FieldTarget> {
        self.current_field().map(|field| field.target)
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
        Line::from("Enter save   Esc cancel   Backspace delete"),
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

    let footer = if picker.custom_mode {
        "Enter apply   Esc list/cancel   Backspace delete"
    } else {
        "Enter select   C custom   type a number to enter custom   Q cancel"
    };
    frame.render_widget(Paragraph::new(footer), chunks[2]);
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

fn menu_entries() -> Vec<MenuEntry> {
    let mut entries = Vec::with_capacity(MENU_LEN);
    entries.push(MenuEntry::Policy);
    entries.extend(TOOL_NAMES.iter().copied().map(MenuEntry::Tool));
    entries.push(MenuEntry::Homebrew);
    entries.push(MenuEntry::Npm);
    entries.push(MenuEntry::Preview);
    entries
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
            let marker = if enabled { "[x]" } else { "[ ]" };
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
        MenuEntry::Preview => "Preview",
    }
}

fn field_rows(draft: &InitDraft, entry: MenuEntry) -> Vec<FieldRow> {
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
        MenuEntry::Preview => Vec::new(),
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
    }
}

fn tool_enabled(draft: &InitDraft, tool: &str) -> bool {
    match tool {
        "fnm" => draft.fnm.is_some(),
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
        "fnm" => set_simple_tool(&mut draft.fnm, enabled, "brew"),
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
        "bun" => set_simple_tool(&mut draft.bun, enabled, "brew"),
        "deno" => set_simple_tool(&mut draft.deno, enabled, "brew"),
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
        "python" => set_simple_tool(&mut draft.python, enabled, "brew"),
        "poetry" => set_simple_tool(&mut draft.poetry, enabled, "brew"),
        "ruby" => set_simple_tool(&mut draft.ruby, enabled, "brew"),
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
    GoDraft {
        version: "stable".to_string(),
        manager: "brew".to_string(),
        source: "brew".to_string(),
        install_dir: None,
    }
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
    if delta.is_negative() {
        index.saturating_sub(delta.unsigned_abs())
    } else {
        (index + delta as usize).min(len.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::init::{InitDraft, PolicyDraft};
    use crate::latest::{VersionCandidate, VersionCandidates};

    use super::{
        FieldTarget, Focus, InitTuiApp, VersionPickerState, apply_field_edit, default_node_draft,
        parse_list, set_tool_enabled, tool_enabled,
    };

    #[test]
    fn toggles_tools_with_defaults() {
        let mut draft = empty_draft();

        set_tool_enabled(&mut draft, "node", true);
        set_tool_enabled(&mut draft, "bun", true);
        set_tool_enabled(&mut draft, "go", true);

        assert!(tool_enabled(&draft, "node"));
        assert_eq!(draft.node.as_ref().unwrap().manager, "fnm");
        assert_eq!(draft.bun.as_ref().unwrap().manager, "brew");
        assert_eq!(draft.go.as_ref().unwrap().source, "brew");

        set_tool_enabled(&mut draft, "node", false);
        assert!(!tool_enabled(&draft, "node"));
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

    fn empty_draft() -> InitDraft {
        InitDraft {
            policy: PolicyDraft {
                channel: "stable".to_string(),
                platform: "macos-arm64".to_string(),
            },
            fnm: None,
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
}
