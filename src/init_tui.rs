use std::{
    io,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
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
    doctor::{DoctorReport, IssueEvidence, IssueSeverity, Status, ToolStatus, build_doctor_report},
    i18n::{Label, Language, Messages, Text},
    init::{
        CliDraft, GoDraft, HomebrewDraft, InitDraft, InitInteractionOutcome, InitWriteResult,
        NodeDraft, NpmDraft, RustDraft, SimpleToolDraft, render_init_document, write_init_document,
    },
    latest::{VersionCandidate, VersionCandidates, lookup_version_candidates},
    platform::OperatingSystem,
    sync::{
        SyncExecution, SyncPlan, SyncStepExecutionStatus, SyncStepKind, build_sync_plan,
        execute_sync_plan_with_progress,
    },
};

const TOOL_NAMES: &[&str] = &[
    "fnm", "nvm", "node", "npm", "pnpm", "yarn", "bun", "deno", "go", "rust", "uv", "python",
    "poetry", "ruby", "wrangler",
];
const NODE_PACKAGE_MANAGER_TOOLS: &[&str] = &["npm", "pnpm", "yarn", "bun"];

const MENU_LEN: usize = TOOL_NAMES.len() + 5;
const ENABLED_DOT: &str = "●";
const DISABLED_DOT: &str = "○";

#[derive(Debug, Clone)]
pub struct InitTuiOptions {
    pub output: PathBuf,
    pub force: bool,
    pub stdout: bool,
    pub language: Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Menu,
    Fields,
    SidePanel,
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
    progress: ActionProgress,
    receiver: Receiver<ActionTaskMessage>,
}

#[derive(Debug, Clone)]
struct ActionProgress {
    label: String,
    detail: Option<String>,
    current: Option<usize>,
    total: Option<usize>,
}

enum ActionTaskMessage {
    Progress(ActionProgress),
    Finished(std::result::Result<ActionOutput, String>),
}

#[derive(Clone)]
struct ActionProgressReporter {
    sender: Sender<ActionTaskMessage>,
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

#[derive(Debug, Clone, Copy)]
enum TuiText {
    ReadyStatus,
    ActionsEmpty,
    PreviewEmpty,
    NoEditableFields,
    Full,
    Back,
    Scroll,
    Page,
    Save,
    Done,
    Quit,
    Move,
    Pane,
    Run,
    Toggle,
    Edit,
    Language,
    ChooseAction,
    StdoutPreview,
    ActionRunningWait,
    EscIgnored,
    ReturnHint,
    SidePanelHelp,
    VersionLookupCancelled,
    CustomVersionPrompt,
    CloseEditHint,
    VersionEditCancelled,
    ReturnedToVersionList,
    CustomVersionEmpty,
    CloseVersionPickerHint,
    ApplyEditHint,
    EditCancelled,
    SelectAction,
    FullPreviewStatus,
    ReturnedEditor,
    ActionsFocused,
    PreviewFocused,
    ReturnedFields,
    FullActionOutputStatus,
    ReturnedActions,
    ConfirmApply,
    ConfirmApplyStatus,
    ApplyConfirmHint,
    FinishFailed,
    Starting,
    Finished,
    FinishedWithIssues,
    Failed,
    Cancelled,
    RenderingEffectiveConfig,
    InspectingLocalTools,
    FormattingDoctorReport,
    BuildingSyncGraph,
    FormattingSyncPreview,
    RenderingToml,
    WritingOutput,
    FormattingSyncExecution,
    LoadingVersions,
    LoadingVersionsFor,
    CurrentValue,
    CustomSelectorNow,
    CustomVersionSelector,
    Source,
    Version,
    NodePackageManagers,
    EnterSaveCancel,
    EnterApplyCancel,
    EnterApplyList,
    EnterSelectCustom,
    ApplyCancel,
    Working,
    Step,
    ShowGeneratedToml,
    DoctorAgainstPolicy,
    DryRunPlan,
    InstallConfigureNow,
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

fn tui_text(messages: Messages, text: TuiText) -> &'static str {
    match messages.language() {
        Language::En => match text {
            TuiText::ReadyStatus => "Ready. Edit the policy, then use Actions to check or sync.",
            TuiText::ActionsEmpty => {
                "Run actions from this TUI: save the policy, check the machine, preview sync, or apply sync."
            }
            TuiText::PreviewEmpty => {
                "Press Enter, Tab, or P to open the full TOML preview.\n\nPress F to finish, S to save, or Q to cancel."
            }
            TuiText::NoEditableFields => "No editable fields for this section.",
            TuiText::Full => "full",
            TuiText::Back => "back",
            TuiText::Scroll => "scroll",
            TuiText::Page => "page",
            TuiText::Save => "save",
            TuiText::Done => "done",
            TuiText::Quit => "quit",
            TuiText::Move => "move",
            TuiText::Pane => "pane",
            TuiText::Run => "run",
            TuiText::Toggle => "toggle",
            TuiText::Edit => "edit",
            TuiText::Language => "lang",
            TuiText::ChooseAction => "Choose an action in the center pane.",
            TuiText::StdoutPreview => "stdout preview",
            TuiText::ActionRunningWait => "An action is running; wait for it to finish.",
            TuiText::EscIgnored => "Use Q to quit; Esc is ignored to protect arrow-key prefixes.",
            TuiText::ReturnHint => "Use Tab/P to return, Q to quit.",
            TuiText::SidePanelHelp => {
                "Tab/P focuses Preview or Actions output; Up/Dn scrolls it. L switches language."
            }
            TuiText::VersionLookupCancelled => "Version lookup cancelled",
            TuiText::CustomVersionPrompt => "Enter a custom version selector",
            TuiText::CloseEditHint => "Use Ctrl+G to close this edit; Esc is ignored.",
            TuiText::VersionEditCancelled => "Version edit cancelled",
            TuiText::ReturnedToVersionList => "Returned to version list",
            TuiText::CustomVersionEmpty => "Custom version cannot be empty",
            TuiText::CloseVersionPickerHint => "Use Q to close the version picker",
            TuiText::ApplyEditHint => "Use Enter to apply this edit; Esc is ignored.",
            TuiText::EditCancelled => "Edit cancelled",
            TuiText::SelectAction => "Select an action and press Enter.",
            TuiText::FullPreviewStatus => {
                "Full preview: arrows scroll, PgUp/PgDn page, Tab/P returns"
            }
            TuiText::ReturnedEditor => "Returned to editor. Tab/P opens preview again.",
            TuiText::ActionsFocused => {
                "Actions output focused. Up/Dn scrolls, Enter opens full output."
            }
            TuiText::PreviewFocused => "Preview focused. Up/Dn scrolls, Enter opens full preview.",
            TuiText::ReturnedFields => "Returned to fields. Left moves to Sections.",
            TuiText::FullActionOutputStatus => {
                "Full action output: arrows scroll, PgUp/PgDn page, Tab/P returns"
            }
            TuiText::ReturnedActions => "Returned to actions. Tab/P opens full output again.",
            TuiText::ConfirmApply => {
                "This can run install commands and update managed shell snippets."
            }
            TuiText::ConfirmApplyStatus => "Confirm apply sync from the popup.",
            TuiText::ApplyConfirmHint => "Use A to apply or Q to cancel; Esc is ignored.",
            TuiText::FinishFailed => "Finish failed; fix the output path or use --force.",
            TuiText::Starting => "Starting",
            TuiText::Finished => "finished",
            TuiText::FinishedWithIssues => "finished with issues",
            TuiText::Failed => "failed",
            TuiText::Cancelled => "cancelled",
            TuiText::RenderingEffectiveConfig => "Rendering effective config",
            TuiText::InspectingLocalTools => "Inspecting local tools",
            TuiText::FormattingDoctorReport => "Formatting doctor report",
            TuiText::BuildingSyncGraph => "Building sync dependency graph",
            TuiText::FormattingSyncPreview => "Formatting sync preview",
            TuiText::RenderingToml => "Rendering TOML",
            TuiText::WritingOutput => "Writing output",
            TuiText::FormattingSyncExecution => "Formatting sync execution",
            TuiText::LoadingVersions => "Loading versions",
            TuiText::LoadingVersionsFor => "Loading versions for",
            TuiText::CurrentValue => "Current value",
            TuiText::CustomSelectorNow => "Press C to type a custom selector now, or Q to cancel.",
            TuiText::CustomVersionSelector => "Custom version selector",
            TuiText::Source => "Source",
            TuiText::Version => "Version",
            TuiText::NodePackageManagers => "node package",
            TuiText::EnterSaveCancel => "Enter save   Ctrl+G cancel   Backspace delete",
            TuiText::EnterApplyCancel => "Enter apply   Ctrl+G cancel   Backspace delete",
            TuiText::EnterApplyList => "Enter apply   Ctrl+G list   Backspace delete",
            TuiText::EnterSelectCustom => {
                "Enter select   C custom   type a number to enter custom   Q cancel"
            }
            TuiText::ApplyCancel => "A apply   Q cancel",
            TuiText::Working => "Working",
            TuiText::Step => "Step",
            TuiText::ShowGeneratedToml => "show generated TOML",
            TuiText::DoctorAgainstPolicy => "doctor against current policy",
            TuiText::DryRunPlan => "dry-run plan",
            TuiText::InstallConfigureNow => "install/configure now",
        },
        Language::Zh => match text {
            TuiText::ReadyStatus => "就绪。编辑策略后，可在操作中检查或同步。",
            TuiText::ActionsEmpty => "可在此执行保存策略、检查当前机器、预览同步或执行同步。",
            TuiText::PreviewEmpty => {
                "按 Enter、Tab 或 P 打开完整 TOML 预览。\n\n按 F 完成，S 保存，Q 退出。"
            }
            TuiText::NoEditableFields => "此区域没有可编辑字段。",
            TuiText::Full => "完整",
            TuiText::Back => "返回",
            TuiText::Scroll => "滚动",
            TuiText::Page => "翻页",
            TuiText::Save => "保存",
            TuiText::Done => "完成",
            TuiText::Quit => "退出",
            TuiText::Move => "移动",
            TuiText::Pane => "面板",
            TuiText::Run => "运行",
            TuiText::Toggle => "切换",
            TuiText::Edit => "编辑",
            TuiText::Language => "语言",
            TuiText::ChooseAction => "请在中间面板选择一个操作。",
            TuiText::StdoutPreview => "stdout 预览",
            TuiText::ActionRunningWait => "操作正在运行，请等待完成。",
            TuiText::EscIgnored => "按 Q 退出；Esc 会被忽略，以避免误判方向键前缀。",
            TuiText::ReturnHint => "按 Tab/P 返回，Q 退出。",
            TuiText::SidePanelHelp => "Tab/P 聚焦预览或操作输出；Up/Dn 滚动。L 切换语言。",
            TuiText::VersionLookupCancelled => "已取消版本查询",
            TuiText::CustomVersionPrompt => "请输入自定义版本选择器",
            TuiText::CloseEditHint => "按 Ctrl+G 关闭编辑；Esc 会被忽略。",
            TuiText::VersionEditCancelled => "已取消版本编辑",
            TuiText::ReturnedToVersionList => "已返回版本列表",
            TuiText::CustomVersionEmpty => "自定义版本不能为空",
            TuiText::CloseVersionPickerHint => "按 Q 关闭版本选择器",
            TuiText::ApplyEditHint => "按 Enter 应用编辑；Esc 会被忽略。",
            TuiText::EditCancelled => "已取消编辑",
            TuiText::SelectAction => "请选择操作并按 Enter。",
            TuiText::FullPreviewStatus => "完整预览：方向键滚动，PgUp/PgDn 翻页，Tab/P 返回",
            TuiText::ReturnedEditor => "已返回编辑器。Tab/P 可再次打开预览。",
            TuiText::ActionsFocused => "已聚焦操作输出。Up/Dn 滚动，Enter 打开完整输出。",
            TuiText::PreviewFocused => "已聚焦预览。Up/Dn 滚动，Enter 打开完整预览。",
            TuiText::ReturnedFields => "已返回字段面板。Left 移动到区域列表。",
            TuiText::FullActionOutputStatus => {
                "完整操作输出：方向键滚动，PgUp/PgDn 翻页，Tab/P 返回"
            }
            TuiText::ReturnedActions => "已返回操作。Tab/P 可再次打开完整输出。",
            TuiText::ConfirmApply => "这会运行安装命令，并更新受管理的 shell 配置片段。",
            TuiText::ConfirmApplyStatus => "请在弹窗中确认执行同步。",
            TuiText::ApplyConfirmHint => "按 A 执行或 Q 取消；Esc 会被忽略。",
            TuiText::FinishFailed => "完成失败；请修复输出路径或使用 --force。",
            TuiText::Starting => "启动中",
            TuiText::Finished => "已完成",
            TuiText::FinishedWithIssues => "已完成但存在问题",
            TuiText::Failed => "失败",
            TuiText::Cancelled => "已取消",
            TuiText::RenderingEffectiveConfig => "正在生成有效配置",
            TuiText::InspectingLocalTools => "正在检查本地工具",
            TuiText::FormattingDoctorReport => "正在格式化检查报告",
            TuiText::BuildingSyncGraph => "正在构建同步依赖图",
            TuiText::FormattingSyncPreview => "正在格式化同步预览",
            TuiText::RenderingToml => "正在生成 TOML",
            TuiText::WritingOutput => "正在写入输出",
            TuiText::FormattingSyncExecution => "正在格式化同步结果",
            TuiText::LoadingVersions => "正在加载版本",
            TuiText::LoadingVersionsFor => "正在加载版本：",
            TuiText::CurrentValue => "当前值",
            TuiText::CustomSelectorNow => "按 C 立即输入自定义选择器，或按 Q 取消。",
            TuiText::CustomVersionSelector => "自定义版本选择器",
            TuiText::Source => "来源",
            TuiText::Version => "版本",
            TuiText::NodePackageManagers => "Node 包",
            TuiText::EnterSaveCancel => "Enter 保存   Ctrl+G 取消   Backspace 删除",
            TuiText::EnterApplyCancel => "Enter 应用   Ctrl+G 取消   Backspace 删除",
            TuiText::EnterApplyList => "Enter 应用   Ctrl+G 列表   Backspace 删除",
            TuiText::EnterSelectCustom => "Enter 选择   C 自定义   输入数字进入自定义   Q 取消",
            TuiText::ApplyCancel => "A 执行   Q 取消",
            TuiText::Working => "处理中",
            TuiText::Step => "步骤",
            TuiText::ShowGeneratedToml => "显示生成的 TOML",
            TuiText::DoctorAgainstPolicy => "按当前策略运行 doctor",
            TuiText::DryRunPlan => "预览计划",
            TuiText::InstallConfigureNow => "立即安装/配置",
        },
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
                language: Language::En,
            },
        )
    }

    fn with_options(mut draft: InitDraft, options: InitTuiOptions) -> Self {
        normalize_node_package_manager_state(&mut draft);
        let messages = Messages::new(options.language);
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
            status: tui_text(messages, TuiText::ReadyStatus).to_string(),
            preview_scroll: 0,
            preview_expanded: false,
            action_expanded: false,
            saved_once: false,
            dirty: true,
        }
    }

    fn messages(&self) -> Messages {
        Messages::new(self.options.language)
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

        let messages = self.messages();
        if let Some(edit) = &self.edit {
            render_edit_popup(frame, area, edit, messages);
        }
        if let Some(fetch) = &self.version_fetch {
            render_version_loading_popup(frame, area, fetch, messages);
        }
        if let Some(picker) = &self.version_picker {
            render_version_picker(frame, area, picker, messages);
        }
        if let Some(confirm) = &self.confirm {
            render_confirm_popup(frame, area, confirm, messages);
        }
        if let Some(task) = &self.action_task {
            render_action_loading_popup(frame, area, task, messages);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let messages = self.messages();
        let enabled = TOOL_NAMES
            .iter()
            .filter(|tool| tool_enabled(&self.draft, tool))
            .count();
        let state = if self.action_task.is_some() {
            (messages.text(Text::Running), Color::Yellow)
        } else if self.dirty {
            (messages.text(Text::Unsaved), Color::Yellow)
        } else {
            (messages.text(Text::Saved), Color::Green)
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
                Span::raw(format!("  {}  ", messages.text(Text::PolicyBuilder))),
                Span::styled(
                    format!("[{}]", state.0),
                    Style::default().fg(state.1).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::raw(format!("{} ", messages.text(Text::Channel))),
                Span::styled(
                    &self.draft.policy.channel,
                    Style::default().fg(Color::Green),
                ),
                Span::raw(format!("  {} ", messages.text(Text::Platform))),
                Span::styled(
                    &self.draft.policy.platform,
                    Style::default().fg(Color::Green),
                ),
                Span::raw(format!(
                    "  {} {enabled}/{}",
                    messages.text(Text::EnabledTools),
                    TOOL_NAMES.len()
                )),
                Span::raw(format!("  {} ", messages.text(Text::Output))),
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
        let messages = self.messages();
        let items = menu_entries()
            .into_iter()
            .map(|entry| ListItem::new(menu_line(&self.draft, entry, messages)))
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
                    .title(format!(" {} ", messages.text(Text::Sections)))
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
        let messages = self.messages();
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
            .title(format!(" {} ", entry_title(entry, messages)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .padding(Padding::horizontal(1));

        if fields.is_empty() {
            let text = match entry {
                MenuEntry::Actions => tui_text(messages, TuiText::ActionsEmpty),
                MenuEntry::Preview => tui_text(messages, TuiText::PreviewEmpty),
                _ => tui_text(messages, TuiText::NoEditableFields),
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
        let messages = self.messages();
        let preview = render_init_document(&self.draft).content;
        let title = if expanded {
            format!(
                " {} - {} (Tab/P {}, PgUp/PgDn {}) ",
                messages.text(Text::Preview),
                tui_text(messages, TuiText::Full),
                tui_text(messages, TuiText::Back),
                tui_text(messages, TuiText::Scroll)
            )
        } else {
            format!(" {} ", messages.text(Text::Preview))
        };
        let border = if expanded
            || (self.focus == Focus::SidePanel
                && !matches!(current_menu_entry(self.menu_index), MenuEntry::Actions))
        {
            Color::Magenta
        } else {
            Color::DarkGray
        };
        let paragraph = Paragraph::new(preview)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border)),
            )
            .scroll((self.preview_scroll, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_action_output(&self, frame: &mut Frame, area: Rect, expanded: bool) {
        let messages = self.messages();
        let output_label = if self.options.stdout {
            tui_text(messages, TuiText::StdoutPreview).to_string()
        } else {
            self.options.output.to_string_lossy().to_string()
        };
        let (title, lines, ok) = match &self.action_output {
            Some(output) => {
                let title = if expanded {
                    format!(
                        " {} - {} - {} (Tab/P {}, PgUp/PgDn {}) ",
                        messages.text(Text::Actions),
                        tui_text(messages, TuiText::Full),
                        output.title,
                        tui_text(messages, TuiText::Back),
                        tui_text(messages, TuiText::Scroll)
                    )
                } else {
                    format!(" {} - {} ", messages.text(Text::Actions), output.title)
                };
                (title, output.lines.clone(), output.ok)
            }
            None => (
                if expanded {
                    format!(
                        " {} - {} (Tab/P {}) ",
                        messages.text(Text::Actions),
                        tui_text(messages, TuiText::Full),
                        tui_text(messages, TuiText::Back)
                    )
                } else {
                    format!(" {} ", messages.text(Text::Actions))
                },
                vec![
                    format!("{}: {output_label}", messages.text(Text::Output)),
                    String::new(),
                    tui_text(messages, TuiText::ChooseAction).to_string(),
                ],
                true,
            ),
        };
        let border = if !ok {
            Color::Red
        } else if self.focus == Focus::SidePanel
            && matches!(current_menu_entry(self.menu_index), MenuEntry::Actions)
        {
            Color::Green
        } else {
            Color::DarkGray
        };
        let paragraph = Paragraph::new(action_output_lines(&lines))
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
        let messages = self.messages();
        let keys = if self.preview_expanded || self.action_expanded {
            Line::from(vec![
                Span::styled("Up/Dn", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Scroll))),
                Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Page))),
                Span::styled("Tab/P", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Back))),
                Span::styled("S", Style::default().fg(Color::Green)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Save))),
                Span::styled("F", Style::default().fg(Color::Green)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Done))),
                Span::styled("L", Style::default().fg(Color::Magenta)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Language))),
                Span::styled("Q", Style::default().fg(Color::Red)),
                Span::raw(format!(" {}", tui_text(messages, TuiText::Quit))),
            ])
        } else if self.focus == Focus::SidePanel {
            let target = if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
                messages.text(Text::Output)
            } else {
                messages.text(Text::Preview)
            };
            Line::from(vec![
                Span::styled("Up/Dn", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Scroll))),
                Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Page))),
                Span::styled("Tab/P", Style::default().fg(Color::Magenta)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Back))),
                Span::styled("Enter", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {} ", tui_text(messages, TuiText::Full))),
                Span::raw(target),
                Span::raw("  "),
                Span::styled("←/→", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Pane))),
                Span::styled("L", Style::default().fg(Color::Magenta)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Language))),
                Span::styled("Q", Style::default().fg(Color::Red)),
                Span::raw(format!(" {}", tui_text(messages, TuiText::Quit))),
            ])
        } else if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
            Line::from(vec![
                Span::styled("Up/Dn", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Move))),
                Span::styled("←/→", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Pane))),
                Span::styled("Enter", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Run))),
                Span::styled("Tab/P", Style::default().fg(Color::Magenta)),
                Span::raw(format!(" {}  ", messages.text(Text::Output))),
                Span::styled("S", Style::default().fg(Color::Green)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Save))),
                Span::styled("F", Style::default().fg(Color::Green)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Done))),
                Span::styled("L", Style::default().fg(Color::Magenta)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Language))),
                Span::styled("Q", Style::default().fg(Color::Red)),
                Span::raw(format!(" {}", tui_text(messages, TuiText::Quit))),
            ])
        } else {
            Line::from(vec![
                Span::styled("Up/Dn", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Move))),
                Span::styled("←/→", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Pane))),
                Span::styled("Space", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Toggle))),
                Span::styled("Enter", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Edit))),
                Span::styled("Tab/P", Style::default().fg(Color::Magenta)),
                Span::raw(format!(" {}  ", messages.text(Text::Preview))),
                Span::styled("S", Style::default().fg(Color::Green)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Save))),
                Span::styled("F", Style::default().fg(Color::Green)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Done))),
                Span::styled("L", Style::default().fg(Color::Magenta)),
                Span::raw(format!(" {}  ", tui_text(messages, TuiText::Language))),
                Span::styled("Q", Style::default().fg(Color::Red)),
                Span::raw(format!(" {}", tui_text(messages, TuiText::Quit))),
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
        let messages = self.messages();
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if self.action_task.is_some() {
                self.status = tui_text(messages, TuiText::ActionRunningWait).to_string();
                return None;
            }
            return Some(AppExit::Cancelled);
        }

        if self.confirm.is_some() {
            return self.handle_confirm_key(key);
        }

        if self.action_task.is_some() {
            self.status = tui_text(messages, TuiText::ActionRunningWait).to_string();
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
                self.status = tui_text(messages, TuiText::EscIgnored).to_string();
                None
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.start_save_action();
                None
            }
            KeyCode::Char('f') | KeyCode::Char('F') => self.finish(),
            KeyCode::Char('l') | KeyCode::Char('L') => {
                self.toggle_language();
                None
            }
            KeyCode::Tab => {
                self.toggle_side_panel_focus();
                None
            }
            KeyCode::Right => {
                self.focus = match self.focus {
                    Focus::Menu => Focus::Fields,
                    Focus::Fields | Focus::SidePanel => Focus::SidePanel,
                };
                None
            }
            KeyCode::Left => {
                self.focus = match self.focus {
                    Focus::SidePanel => Focus::Fields,
                    Focus::Fields | Focus::Menu => Focus::Menu,
                };
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.focus == Focus::SidePanel {
                    self.scroll_active_panel(-1);
                } else {
                    self.move_selection(-1);
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.focus == Focus::SidePanel {
                    self.scroll_active_panel(1);
                } else {
                    self.move_selection(1);
                }
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
                if self.focus == Focus::SidePanel {
                    self.open_context_panel();
                    None
                } else {
                    self.start_edit_or_focus_fields()
                }
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.toggle_side_panel_focus();
                None
            }
            KeyCode::Char('?') => {
                self.status = tui_text(messages, TuiText::SidePanelHelp).to_string();
                None
            }
            _ => None,
        }
    }

    fn handle_preview_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        let messages = self.messages();
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Some(AppExit::Cancelled),
            KeyCode::Esc => {
                self.status = tui_text(messages, TuiText::ReturnHint).to_string();
                None
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.start_save_action();
                None
            }
            KeyCode::Char('f') | KeyCode::Char('F') => self.finish(),
            KeyCode::Char('l') | KeyCode::Char('L') => {
                self.toggle_language();
                None
            }
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
        let messages = self.messages();
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Some(AppExit::Cancelled),
            KeyCode::Esc => {
                self.status = tui_text(messages, TuiText::ReturnHint).to_string();
                None
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.start_save_action();
                None
            }
            KeyCode::Char('f') | KeyCode::Char('F') => self.finish(),
            KeyCode::Char('l') | KeyCode::Char('L') => {
                self.toggle_language();
                None
            }
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
        let messages = self.messages();
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.version_fetch = None;
                self.status = tui_text(messages, TuiText::VersionLookupCancelled).to_string();
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if let Some(fetch) = self.version_fetch.take() {
                    self.version_picker = Some(VersionPickerState::custom_only(
                        fetch.target,
                        fetch.label,
                        fetch.current,
                    ));
                    self.status = tui_text(messages, TuiText::CustomVersionPrompt).to_string();
                }
            }
            _ => {}
        }
        None
    }

    fn handle_version_picker_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        let messages = self.messages();
        let mut picker = self.version_picker.take().expect("version picker checked");

        if picker.custom_mode {
            match key.code {
                KeyCode::Esc => {
                    self.status = tui_text(messages, TuiText::CloseEditHint).to_string();
                    self.version_picker = Some(picker);
                }
                KeyCode::Char('g') | KeyCode::Char('G')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if picker.choices.is_empty() {
                        self.status = tui_text(messages, TuiText::VersionEditCancelled).to_string();
                    } else {
                        picker.custom_mode = false;
                        self.status =
                            tui_text(messages, TuiText::ReturnedToVersionList).to_string();
                        self.version_picker = Some(picker);
                    }
                }
                KeyCode::Enter => {
                    let value = picker.custom_buffer.trim().to_string();
                    if value.is_empty() {
                        self.status = tui_text(messages, TuiText::CustomVersionEmpty).to_string();
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
                self.status = tui_text(messages, TuiText::VersionEditCancelled).to_string();
            }
            KeyCode::Esc => {
                self.status = tui_text(messages, TuiText::CloseVersionPickerHint).to_string();
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
        let messages = self.messages();
        let mut edit = self.edit.take().expect("edit mode checked");
        match key.code {
            KeyCode::Esc => {
                self.status = tui_text(messages, TuiText::ApplyEditHint).to_string();
                self.edit = Some(edit);
            }
            KeyCode::Char('g') | KeyCode::Char('G')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.status = tui_text(messages, TuiText::EditCancelled).to_string();
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
            Focus::SidePanel => {
                self.scroll_active_panel(delta.clamp(i16::MIN as isize, i16::MAX as isize) as i16);
            }
        }
    }

    fn toggle_current_tool_or_field(&mut self) -> Option<AppExit> {
        let messages = self.messages();
        match self.focus {
            Focus::Menu => {
                if let MenuEntry::Tool(tool) = current_menu_entry(self.menu_index) {
                    toggle_tool(&mut self.draft, tool);
                    self.mark_dirty();
                    self.status = format!(
                        "{tool} {}",
                        enabled_label(tool_enabled(&self.draft, tool), messages)
                    );
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
                                enabled_label(tool_enabled(&self.draft, tool), messages)
                            );
                        }
                        FieldTarget::Action(action) => {
                            return self.run_action(action);
                        }
                        _ => {}
                    }
                }
            }
            Focus::SidePanel => {}
        }
        None
    }

    fn start_edit_or_focus_fields(&mut self) -> Option<AppExit> {
        let messages = self.messages();
        if self.focus == Focus::SidePanel {
            self.open_context_panel();
            return None;
        }

        if self.focus == Focus::Menu {
            match current_menu_entry(self.menu_index) {
                MenuEntry::Preview => self.focus_context_panel(),
                _ => {
                    self.focus = Focus::Fields;
                    if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
                        self.status = tui_text(messages, TuiText::SelectAction).to_string();
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
                self.status = format!(
                    "{tool} {}",
                    enabled_label(tool_enabled(&self.draft, tool), messages)
                );
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
        self.status = tui_text(self.messages(), TuiText::FullPreviewStatus).to_string();
    }

    fn close_preview(&mut self) {
        self.preview_expanded = false;
        self.status = tui_text(self.messages(), TuiText::ReturnedEditor).to_string();
    }

    fn focus_context_panel(&mut self) {
        self.preview_expanded = false;
        self.action_expanded = false;
        self.focus = Focus::SidePanel;
        self.status = if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
            tui_text(self.messages(), TuiText::ActionsFocused).to_string()
        } else {
            tui_text(self.messages(), TuiText::PreviewFocused).to_string()
        };
    }

    fn toggle_side_panel_focus(&mut self) {
        if self.focus == Focus::SidePanel {
            self.focus = Focus::Fields;
            self.status = tui_text(self.messages(), TuiText::ReturnedFields).to_string();
        } else {
            self.focus_context_panel();
        }
    }

    fn toggle_language(&mut self) {
        self.options.language = match self.options.language {
            Language::En => Language::Zh,
            Language::Zh => Language::En,
        };
        self.status = match self.options.language {
            Language::En => "Language switched to English".to_string(),
            Language::Zh => "已切换为中文".to_string(),
        };
    }

    fn open_context_panel(&mut self) {
        if matches!(current_menu_entry(self.menu_index), MenuEntry::Actions) {
            self.open_action_output();
        } else {
            self.open_preview();
        }
    }

    fn open_action_output(&mut self) {
        self.action_expanded = true;
        self.preview_expanded = false;
        self.menu_index = action_menu_index();
        self.status = tui_text(self.messages(), TuiText::FullActionOutputStatus).to_string();
    }

    fn close_action_output(&mut self) {
        self.action_expanded = false;
        self.status = tui_text(self.messages(), TuiText::ReturnedActions).to_string();
    }

    fn run_action(&mut self, action: ActionTarget) -> Option<AppExit> {
        let messages = self.messages();
        match action {
            ActionTarget::SaveConfig => {
                self.start_save_action();
                None
            }
            ActionTarget::RunCheck => {
                self.start_action_task(
                    messages.text(Text::CheckEnvironment),
                    |draft, options, progress| {
                        let messages = Messages::new(options.language);
                        progress.update(tui_text(messages, TuiText::RenderingEffectiveConfig));
                        let config = config_from_draft(&draft)?;
                        progress.update_detail(
                            tui_text(messages, TuiText::InspectingLocalTools),
                            "PATH, versions, managers",
                        );
                        let report = build_doctor_report(&config);
                        progress.update(tui_text(messages, TuiText::FormattingDoctorReport));
                        Ok(doctor_output(report, &options))
                    },
                );
                None
            }
            ActionTarget::PreviewSync => {
                self.start_action_task(
                    messages.text(Text::PreviewSync),
                    |draft, options, progress| {
                        let messages = Messages::new(options.language);
                        progress.update(tui_text(messages, TuiText::RenderingEffectiveConfig));
                        let config = config_from_draft(&draft)?;
                        progress.update_detail(
                            tui_text(messages, TuiText::InspectingLocalTools),
                            "PATH, versions, managers",
                        );
                        let report = build_doctor_report(&config);
                        progress.update(tui_text(messages, TuiText::BuildingSyncGraph));
                        let plan = build_sync_plan(true, &options.output, &config, &report);
                        progress.update(tui_text(messages, TuiText::FormattingSyncPreview));
                        Ok(sync_plan_output(plan, messages))
                    },
                );
                None
            }
            ActionTarget::ApplySync => {
                if self.action_task.is_some() {
                    self.status = tui_text(messages, TuiText::ActionRunningWait).to_string();
                } else {
                    self.confirm = Some(ConfirmState {
                        action,
                        title: messages.text(Text::ApplySync).to_string(),
                        message: tui_text(messages, TuiText::ConfirmApply).to_string(),
                    });
                    self.status = tui_text(messages, TuiText::ConfirmApplyStatus).to_string();
                }
                None
            }
        }
    }

    fn start_save_action(&mut self) {
        let messages = self.messages();
        let allow_overwrite = self.options.force || self.saved_once;
        self.start_action_task(
            messages.text(Text::SaveConfig),
            move |draft, options, progress| {
                let messages = Messages::new(options.language);
                progress.update(tui_text(messages, TuiText::RenderingToml));
                progress.update_detail(
                    tui_text(messages, TuiText::WritingOutput),
                    if options.stdout {
                        tui_text(messages, TuiText::StdoutPreview).to_string()
                    } else {
                        options.output.display().to_string()
                    },
                );
                save_config_output(&draft, &options, allow_overwrite)
            },
        );
    }

    fn start_apply_sync_action(&mut self) {
        let messages = self.messages();
        self.start_action_task(
            messages.text(Text::ApplySync),
            |draft, options, progress| {
                let messages = Messages::new(options.language);
                progress.update(tui_text(messages, TuiText::RenderingEffectiveConfig));
                let config = config_from_draft(&draft)?;
                progress.update_detail(
                    tui_text(messages, TuiText::InspectingLocalTools),
                    "PATH, versions, managers",
                );
                let report = build_doctor_report(&config);
                progress.update(tui_text(messages, TuiText::BuildingSyncGraph));
                let plan = build_sync_plan(false, &options.output, &config, &report);
                let execution =
                    execute_sync_plan_with_progress(&plan, &config, |step, current, total| {
                        progress.update_step(
                            current,
                            total,
                            format!("{} {}", kind_label(&step.kind, messages), step.target),
                            step.reason.clone(),
                        );
                    });
                progress.update(tui_text(messages, TuiText::FormattingSyncExecution));
                Ok(sync_execution_output(execution, messages))
            },
        );
    }

    fn start_action_task<F>(&mut self, title: impl Into<String>, work: F)
    where
        F: FnOnce(InitDraft, InitTuiOptions, ActionProgressReporter) -> Result<ActionOutput>
            + Send
            + 'static,
    {
        if self.action_task.is_some() {
            self.status = tui_text(self.messages(), TuiText::ActionRunningWait).to_string();
            return;
        }
        let title = title.into();
        let messages = self.messages();
        self.menu_index = action_menu_index();
        self.focus = Focus::Fields;
        self.preview_expanded = false;
        self.action_expanded = false;
        self.action_scroll = 0;
        let draft = self.draft.clone();
        let options = self.options.clone();
        let (sender, receiver) = mpsc::channel();
        let reporter = ActionProgressReporter {
            sender: sender.clone(),
        };
        thread::spawn(move || {
            let result = work(draft, options, reporter).map_err(|error| error.to_string());
            let _ = sender.send(ActionTaskMessage::Finished(result));
        });
        self.action_task = Some(ActionTaskState {
            title: title.clone(),
            progress: ActionProgress::new(tui_text(messages, TuiText::Starting)),
            receiver,
        });
        self.status = format!("{title} {}...", messages.text(Text::Running));
    }

    fn poll_action_task(&mut self) {
        let messages = self.messages();
        let Some(task) = &mut self.action_task else {
            return;
        };
        let mut finished = None;
        while let Ok(message) = task.receiver.try_recv() {
            match message {
                ActionTaskMessage::Progress(progress) => {
                    self.status = format!("{}: {}", task.title, progress.label);
                    task.progress = progress;
                }
                ActionTaskMessage::Finished(result) => {
                    finished = Some(result);
                    break;
                }
            }
        }
        let Some(result) = finished else { return };
        let title = self.action_task.take().expect("action task checked").title;
        match result {
            Ok(output) => {
                if output.mark_saved {
                    self.saved_once = true;
                    self.dirty = false;
                }
                self.status = if output.ok {
                    format!("{title} {}", tui_text(messages, TuiText::Finished))
                } else {
                    format!(
                        "{title} {}",
                        tui_text(messages, TuiText::FinishedWithIssues)
                    )
                };
                self.action_output = Some(output);
            }
            Err(error) => {
                self.status = format!("{title} {}", tui_text(messages, TuiText::Failed));
                self.action_output = Some(ActionOutput::error(title, error, messages));
            }
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Option<AppExit> {
        let messages = self.messages();
        let confirm = self.confirm.take().expect("confirm checked");
        match key.code {
            KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                if confirm.action == ActionTarget::ApplySync {
                    self.start_apply_sync_action();
                }
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.status = format!(
                    "{} {}",
                    confirm.title,
                    tui_text(messages, TuiText::Cancelled)
                );
            }
            KeyCode::Esc => {
                self.status = tui_text(messages, TuiText::ApplyConfirmHint).to_string();
                self.confirm = Some(confirm);
            }
            _ => {
                self.confirm = Some(confirm);
            }
        }
        None
    }

    fn finish(&mut self) -> Option<AppExit> {
        let messages = self.messages();
        if self.action_task.is_some() {
            self.status = tui_text(messages, TuiText::ActionRunningWait).to_string();
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
                self.action_output = Some(ActionOutput::error(
                    messages.text(Text::Finish).to_string(),
                    error.to_string(),
                    messages,
                ));
                self.status = tui_text(messages, TuiText::FinishFailed).to_string();
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
        self.status = format!(
            "{} {tool}...",
            tui_text(self.messages(), TuiText::LoadingVersions)
        );
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
            tui_text(self.messages(), TuiText::CustomVersionPrompt).to_string()
        } else {
            format!(
                "{} {source}",
                tui_text(self.messages(), TuiText::LoadingVersionsFor)
            )
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
            .map(|output| {
                action_output_line_count(&output.lines)
                    .saturating_sub(1)
                    .min(u16::MAX as usize) as u16
            })
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

fn render_edit_popup(frame: &mut Frame, area: Rect, edit: &EditState, messages: Messages) {
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
        Line::from(tui_text(messages, TuiText::EnterSaveCancel)),
    ];
    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .title(format!(" {} ", messages.text(Text::Edit)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup);
}

fn render_version_loading_popup(
    frame: &mut Frame,
    area: Rect,
    fetch: &VersionFetchState,
    messages: Messages,
) {
    let popup = centered_rect(68, 9, area);
    frame.render_widget(Clear, popup);
    let content = vec![
        Line::from(Span::styled(
            format!(
                "{} {}",
                tui_text(messages, TuiText::LoadingVersionsFor),
                fetch.label
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!(
            "{}: {}",
            tui_text(messages, TuiText::CurrentValue),
            fetch.current
        )),
        Line::from(""),
        Line::from(tui_text(messages, TuiText::CustomSelectorNow)),
    ];
    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .title(format!(" {} ", messages.text(Text::Versions)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, popup);
}

fn render_version_picker(
    frame: &mut Frame,
    area: Rect,
    picker: &VersionPickerState,
    messages: Messages,
) {
    let popup = centered_rect(76, 18, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(format!(
            " {} - {} ",
            tui_text(messages, TuiText::Version),
            picker.label
        ))
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
        Span::raw(format!("{}: ", tui_text(messages, TuiText::Source))),
        Span::styled(picker.source.as_str(), Style::default().fg(Color::Green)),
    ])];
    if let Some(note) = &picker.note {
        header.push(Line::from(note.as_str()));
    }
    frame.render_widget(Paragraph::new(header).wrap(Wrap { trim: true }), chunks[0]);

    if picker.custom_mode {
        let content = vec![
            Line::from(Span::styled(
                tui_text(messages, TuiText::CustomVersionSelector),
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
        tui_text(messages, TuiText::EnterApplyCancel)
    } else if picker.custom_mode {
        tui_text(messages, TuiText::EnterApplyList)
    } else {
        tui_text(messages, TuiText::EnterSelectCustom)
    };
    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

fn render_confirm_popup(frame: &mut Frame, area: Rect, confirm: &ConfirmState, messages: Messages) {
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
        Line::from(tui_text(messages, TuiText::ApplyCancel)),
    ];
    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .title(format!(" {} ", messages.text(Text::Confirm)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, popup);
}

fn render_action_loading_popup(
    frame: &mut Frame,
    area: Rect,
    task: &ActionTaskState,
    messages: Messages,
) {
    let popup = centered_rect(68, 9, area);
    frame.render_widget(Clear, popup);
    let mut content = vec![
        Line::from(Span::styled(
            task.title.as_str(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(task.progress.label.as_str()),
    ];
    if let (Some(current), Some(total)) = (task.progress.current, task.progress.total) {
        content.push(Line::from(format!(
            "{} {current}/{total}",
            tui_text(messages, TuiText::Step)
        )));
    }
    if let Some(detail) = &task.progress.detail {
        content.push(Line::from(detail.as_str()));
    }
    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .title(format!(" {} ", tui_text(messages, TuiText::Working)))
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

    fn saved(result: InitWriteResult, messages: Messages) -> Self {
        let action = if result.overwritten {
            messages.text(Text::Overwrote)
        } else {
            messages.text(Text::Wrote)
        };
        Self {
            title: messages.text(Text::SavedConfig).to_string(),
            lines: vec![format!("{action} {}", result.path.display())],
            ok: true,
            mark_saved: true,
        }
    }

    fn error(title: String, error: String, messages: Messages) -> Self {
        Self {
            title,
            lines: vec![messages.text(Text::Error).to_string(), String::new(), error],
            ok: false,
            mark_saved: false,
        }
    }
}

impl ActionProgress {
    fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: None,
            current: None,
            total: None,
        }
    }

    fn with_detail(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: Some(detail.into()),
            current: None,
            total: None,
        }
    }

    fn step(
        current: usize,
        total: usize,
        label: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            detail: Some(detail.into()),
            current: Some(current),
            total: Some(total),
        }
    }
}

impl ActionProgressReporter {
    fn update(&self, label: impl Into<String>) {
        let progress = ActionProgress::new(label);
        let _ = self.sender.send(ActionTaskMessage::Progress(progress));
    }

    fn update_detail(&self, label: impl Into<String>, detail: impl Into<String>) {
        let progress = ActionProgress::with_detail(label, detail);
        let _ = self.sender.send(ActionTaskMessage::Progress(progress));
    }

    fn update_step(
        &self,
        current: usize,
        total: usize,
        label: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let progress = ActionProgress::step(current, total, label, detail);
        let _ = self.sender.send(ActionTaskMessage::Progress(progress));
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

fn menu_line(draft: &InitDraft, entry: MenuEntry, messages: Messages) -> Line<'static> {
    match entry {
        MenuEntry::Policy => Line::from(vec![
            Span::styled(
                messages.text(Text::Policy),
                Style::default().fg(Color::Yellow),
            ),
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
                Span::raw(format!("  {}", tool_summary(draft, tool, messages))),
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
            Span::styled(
                messages.text(Text::NpmGlobals),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(format!(
                "  {}",
                package_count(draft.npm_config.as_ref().map(|npm| &npm.global_packages))
            )),
        ]),
        MenuEntry::Actions => Line::from(vec![
            Span::styled(
                messages.text(Text::Actions),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  save / check / sync"),
        ]),
        MenuEntry::Preview => Line::from(vec![Span::styled(
            messages.text(Text::Preview),
            Style::default().fg(Color::Magenta),
        )]),
    }
}

fn entry_title(entry: MenuEntry, messages: Messages) -> String {
    match entry {
        MenuEntry::Policy => messages.text(Text::Policy).to_string(),
        MenuEntry::Tool(tool) => tool.to_string(),
        MenuEntry::Homebrew => messages.text(Text::HomebrewPackages).to_string(),
        MenuEntry::Npm => messages.text(Text::NpmGlobals).to_string(),
        MenuEntry::Actions => messages.text(Text::Actions).to_string(),
        MenuEntry::Preview => messages.text(Text::Preview).to_string(),
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
        FieldTarget::Action(ActionTarget::SaveConfig) => Color::Green,
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
        FieldTarget::Action(ActionTarget::SaveConfig) => Color::Green,
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
    let messages = Messages::new(options.language);
    match entry {
        MenuEntry::Policy => vec![
            FieldRow {
                label: messages.text(Text::Channel).to_string(),
                value: draft.policy.channel.clone(),
                target: FieldTarget::PolicyChannel,
            },
            FieldRow {
                label: messages.text(Text::Platform).to_string(),
                value: draft.policy.platform.clone(),
                target: FieldTarget::PolicyPlatform,
            },
        ],
        MenuEntry::Tool(tool) => tool_field_rows(draft, tool, messages),
        MenuEntry::Homebrew => vec![FieldRow {
            label: messages.text(Text::PackageList).to_string(),
            value: list_value(draft.cli.as_ref().map(|cli| &cli.packages)),
            target: FieldTarget::HomebrewPackages,
        }],
        MenuEntry::Npm => vec![FieldRow {
            label: messages.text(Text::PackageList).to_string(),
            value: list_value(draft.npm_config.as_ref().map(|npm| &npm.global_packages)),
            target: FieldTarget::NpmGlobals,
        }],
        MenuEntry::Actions => action_field_rows(options),
        MenuEntry::Preview => Vec::new(),
    }
}

fn action_field_rows(options: &InitTuiOptions) -> Vec<FieldRow> {
    let messages = Messages::new(options.language);
    let output = if options.stdout {
        tui_text(messages, TuiText::ShowGeneratedToml).to_string()
    } else {
        options.output.display().to_string()
    };
    vec![
        FieldRow {
            label: messages.text(Text::SaveConfig).to_string(),
            value: output,
            target: FieldTarget::Action(ActionTarget::SaveConfig),
        },
        FieldRow {
            label: messages.text(Text::RunCheck).to_string(),
            value: tui_text(messages, TuiText::DoctorAgainstPolicy).to_string(),
            target: FieldTarget::Action(ActionTarget::RunCheck),
        },
        FieldRow {
            label: messages.text(Text::PreviewSync).to_string(),
            value: tui_text(messages, TuiText::DryRunPlan).to_string(),
            target: FieldTarget::Action(ActionTarget::PreviewSync),
        },
        FieldRow {
            label: messages.text(Text::ApplySync).to_string(),
            value: tui_text(messages, TuiText::InstallConfigureNow).to_string(),
            target: FieldTarget::Action(ActionTarget::ApplySync),
        },
    ]
}

fn save_config_output(
    draft: &InitDraft,
    options: &InitTuiOptions,
    force: bool,
) -> Result<ActionOutput> {
    let document = render_init_document(draft);
    let messages = Messages::new(options.language);
    if options.stdout {
        let mut lines = vec![
            messages.text(Text::GeneratedToml).to_string(),
            tui_text(messages, TuiText::ShowGeneratedToml).to_string(),
            String::new(),
        ];
        lines.extend(document.content.lines().map(ToString::to_string));
        return Ok(ActionOutput::ok(messages.text(Text::GeneratedToml), lines));
    }

    let result = write_init_document(&options.output, force, &document)?;
    Ok(ActionOutput::saved(result, messages))
}

fn config_from_draft(draft: &InitDraft) -> Result<DevkitConfig> {
    let content = render_init_document(draft).content;
    let config =
        toml::from_str::<DevkitConfig>(&content).context("generated TOML could not be parsed")?;
    Ok(config.effective_for_current_platform())
}

fn doctor_output(report: DoctorReport, options: &InitTuiOptions) -> ActionOutput {
    let messages = Messages::new(options.language);
    let ok_count = report
        .tools
        .iter()
        .filter(|tool| matches!(tool.status, Status::Ok))
        .count();
    let missing_count = report
        .tools
        .iter()
        .filter(|tool| matches!(tool.status, Status::Missing))
        .count();
    let mismatch_count = report
        .tools
        .iter()
        .filter(|tool| matches!(tool.status, Status::Mismatch))
        .count();
    let unknown_count = report
        .tools
        .iter()
        .filter(|tool| matches!(tool.status, Status::Unknown))
        .count();
    let mut lines = vec![
        messages.text(Text::DoctorReport).to_string(),
        format!(
            "{}: {}",
            tui_text(messages, TuiText::Source),
            if options.stdout {
                "current TUI draft".to_string()
            } else {
                options.output.display().to_string()
            }
        ),
        format!(
            "{}: {ok_count} {}, {missing_count} {}, {mismatch_count} {}, {unknown_count} {}",
            messages.text(Text::Summary),
            messages.label(Label::Ok),
            messages.label(Label::Missing),
            messages.label(Label::Mismatch),
            messages.label(Label::Unknown),
        ),
        String::new(),
        messages.text(Text::Tools).to_string(),
    ];
    for tool in &report.tools {
        push_doctor_tool_lines(&mut lines, tool, messages);
    }

    if report.issues.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "{}: {}",
            messages.text(Text::Issues),
            messages.text(Text::None)
        ));
    } else {
        lines.push(String::new());
        lines.push(messages.text(Text::Issues).to_string());
        for issue in &report.issues {
            lines.push(format!(
                "- [{}] {}",
                issue_severity_label(issue.severity, messages),
                issue.message
            ));
            if let Some(path) = &issue.path {
                lines.push(format!(
                    "  {}: {}",
                    messages.text(Text::Path),
                    path.display()
                ));
            }
            for evidence in &issue.evidence {
                push_issue_evidence_lines(&mut lines, evidence, messages);
            }
            if let Some(fix) = &issue.fix {
                lines.push(format!(
                    "  {}: {}",
                    messages.text(Text::Fix),
                    compact_issue_fix(fix, &issue.evidence)
                ));
            }
        }
    }

    let ok = report
        .tools
        .iter()
        .all(|tool| matches!(tool.status, Status::Ok) || tool.required.is_none());
    ActionOutput {
        title: messages.text(Text::CheckEnvironment).to_string(),
        lines,
        ok,
        mark_saved: false,
    }
}

fn push_doctor_tool_lines(lines: &mut Vec<String>, tool: &ToolStatus, messages: Messages) {
    lines.push(format!(
        "- {} [{}]",
        tool.name,
        status_label(&tool.status, messages)
    ));
    lines.push(format!(
        "  {}: {}",
        messages.text(Text::Current),
        tool.current.as_deref().unwrap_or("-")
    ));
    lines.push(format!(
        "  {}: {}",
        messages.text(Text::Required),
        tool.required.as_deref().unwrap_or("-")
    ));
    if let Some(manager) = &tool.manager {
        lines.push(format!("  {}: {manager}", messages.text(Text::Manager)));
    }
    lines.push(format!(
        "  {}: {}",
        messages.text(Text::Command),
        tool.command
    ));
    lines.push(format!(
        "  {}: {}",
        messages.text(Text::Path),
        tool.path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));
    if tool.path_candidates.len() > 1 {
        lines.push(format!(
            "  {}: {}",
            messages.text(Text::PathCandidates),
            tool.path_candidates.len()
        ));
        for (index, path) in tool.path_candidates.iter().enumerate() {
            let active = if index == 0 {
                format!(" ({})", messages.text(Text::Active))
            } else {
                String::new()
            };
            lines.push(format!("    {}. {}{active}", index + 1, path.display()));
        }
    }
    if let Some(note) = &tool.note {
        lines.push(format!("  {}: {note}", messages.text(Text::Note)));
    }
}

fn push_issue_evidence_lines(
    lines: &mut Vec<String>,
    evidence: &IssueEvidence,
    messages: Messages,
) {
    if evidence.key == "candidates" {
        lines.push(format!("  {}:", messages.text(Text::Candidates)));
        for (index, candidate) in evidence.value.split("; ").enumerate() {
            lines.push(format!("    {}. {candidate}", index + 1));
        }
    } else {
        lines.push(format!("  {}: {}", evidence.key, evidence.value));
    }
}

fn compact_issue_fix(fix: &str, evidence: &[IssueEvidence]) -> String {
    if evidence.iter().any(|item| item.key == "candidates")
        && let Some((summary, _)) = fix.split_once("; candidates:")
    {
        return summary.to_string();
    }
    fix.to_string()
}

fn action_output_lines(lines: &[String]) -> Vec<Line<'static>> {
    lines
        .iter()
        .flat_map(|line| line.split('\n').map(styled_action_output_line))
        .collect()
}

fn action_output_line_count(lines: &[String]) -> usize {
    lines.iter().map(|line| line.split('\n').count()).sum()
}

fn styled_action_output_line(line: &str) -> Line<'static> {
    if line.is_empty() {
        return Line::from("");
    }
    if let Some(styled) = styled_bracketed_action_line(line) {
        return styled;
    }
    if let Some(styled) = styled_tool_status_line(line) {
        return styled;
    }
    if let Some(styled) = styled_numbered_action_line(line) {
        return styled;
    }
    if line.starts_with("Summary:") || line.starts_with("摘要:") {
        return styled_summary_line(line);
    }
    if line.starts_with("Result:") || line.starts_with("结果:") {
        return styled_result_line(line);
    }
    if let Some(color) = action_section_color(line) {
        return Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(styled) = styled_key_value_action_line(line) {
        return styled;
    }

    Line::from(Span::raw(line.to_string()))
}

fn styled_bracketed_action_line(line: &str) -> Option<Line<'static>> {
    let rest = line.strip_prefix("- [")?;
    let (label, tail) = rest.split_once(']')?;
    Some(Line::from(vec![
        Span::styled("- [", Style::default().fg(Color::DarkGray)),
        Span::styled(
            label.to_string(),
            status_style(label).add_modifier(Modifier::BOLD),
        ),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
        Span::raw(tail.to_string()),
    ]))
}

fn styled_tool_status_line(line: &str) -> Option<Line<'static>> {
    let rest = line.strip_prefix("- ")?;
    let (name, status) = rest.rsplit_once(" [")?;
    let status = status.strip_suffix(']')?;
    if status.is_empty() {
        return None;
    }

    Some(Line::from(vec![
        Span::styled("- ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            name.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" [", Style::default().fg(Color::DarkGray)),
        Span::styled(
            status.to_string(),
            status_style(status).add_modifier(Modifier::BOLD),
        ),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
    ]))
}

fn styled_numbered_action_line(line: &str) -> Option<Line<'static>> {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];
    let (number, value) = rest.split_once(". ")?;
    if !number.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }

    let mut spans = vec![
        Span::raw(indent.to_string()),
        Span::styled(format!("{number}. "), Style::default().fg(Color::DarkGray)),
    ];
    if let Some((path, marker)) = value
        .strip_suffix(" (active)")
        .map(|path| (path, " (active)"))
        .or_else(|| {
            value
                .strip_suffix(" (当前生效)")
                .map(|path| (path, " (当前生效)"))
        })
    {
        spans.push(Span::styled(
            path.to_string(),
            Style::default().fg(Color::White),
        ));
        spans.push(Span::styled(
            marker,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(
            value.to_string(),
            Style::default().fg(Color::White),
        ));
    }
    Some(Line::from(spans))
}

fn styled_summary_line(line: &str) -> Line<'static> {
    let label = if line.starts_with("摘要:") {
        "摘要:"
    } else {
        "Summary:"
    };
    let rest = line.trim_start_matches(label);
    let mut spans = vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    let rest = rest.trim_start();
    for (index, part) in rest.split(", ").enumerate() {
        if index > 0 {
            spans.push(Span::raw(", "));
        }
        spans.push(Span::styled(
            part.to_string(),
            Style::default().fg(summary_part_color(part)),
        ));
    }

    Line::from(spans)
}

fn styled_result_line(line: &str) -> Line<'static> {
    let label = if line.starts_with("结果:") {
        "结果:"
    } else {
        "Result:"
    };
    let rest = line.trim_start_matches(label);
    let color = if rest.contains("matches policy") || rest.contains("已匹配") {
        Color::Green
    } else {
        Color::Yellow
    };
    Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(rest.to_string(), Style::default().fg(color)),
    ])
}

fn styled_key_value_action_line(line: &str) -> Option<Line<'static>> {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let trimmed = &line[indent_len..];
    if trimmed.starts_with("- ") {
        return None;
    }

    if let Some(key) = trimmed.strip_suffix(':') {
        return Some(Line::from(vec![
            Span::raw(indent.to_string()),
            Span::styled(
                key.to_string(),
                Style::default()
                    .fg(key_value_color(key))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":", Style::default().fg(Color::DarkGray)),
        ]));
    }

    let (key, value) = trimmed.split_once(": ")?;
    Some(Line::from(vec![
        Span::raw(indent.to_string()),
        Span::styled(
            key.to_string(),
            Style::default()
                .fg(key_value_color(key))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(": ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            value.to_string(),
            Style::default().fg(if value == "-" {
                Color::DarkGray
            } else {
                Color::White
            }),
        ),
    ]))
}

fn action_section_color(line: &str) -> Option<Color> {
    match line {
        "Doctor report" | "Tools" | "Generated TOML" | "Sync execution" | "环境检查报告"
        | "工具" | "生成的 TOML" | "同步执行" => Some(Color::Cyan),
        "Issues" | "问题" => Some(Color::Yellow),
        "Issues: none" | "问题: 无" => Some(Color::Green),
        "Ready" | "可执行" => Some(Color::Green),
        "Blocked" | "Error" | "被阻塞" | "错误" => Some(Color::Red),
        "Verify" | "验证" => Some(Color::Magenta),
        _ if line.starts_with("Sync plan") => Some(Color::Cyan),
        _ if line.starts_with("同步计划") => Some(Color::Cyan),
        _ => None,
    }
}

fn status_style(label: &str) -> Style {
    Style::default().fg(status_color(label))
}

fn status_color(label: &str) -> Color {
    match label {
        "正常" | "可执行" | "已执行" | "已验证" | "已启用" => return Color::Green,
        "缺失" | "失败" | "错误" => return Color::Red,
        "不匹配" | "警告" | "已跳过" | "清理" => return Color::Yellow,
        "安装" | "配置" => return Color::Cyan,
        "对齐" => return Color::Magenta,
        "验证" => return Color::Green,
        "信息" | "未变化" => return Color::Blue,
        "未知" | "已禁用" => return Color::DarkGray,
        _ => {}
    }
    match label.to_ascii_lowercase().as_str() {
        "ok" | "ready" | "applied" | "verified" => Color::Green,
        "missing" | "failed" | "error" => Color::Red,
        "mismatch" | "warning" | "skipped" | "cleanup" => Color::Yellow,
        "install" | "configure" => Color::Cyan,
        "align" => Color::Magenta,
        "verify" => Color::Green,
        "info" | "unchanged" => Color::Blue,
        "unknown" => Color::DarkGray,
        _ => Color::White,
    }
}

fn summary_part_color(part: &str) -> Color {
    if part.contains(" ok") || part.contains(" 正常") {
        Color::Green
    } else if part.contains(" missing") || part.contains(" 缺失") {
        Color::Red
    } else if part.contains(" mismatch") || part.contains(" 不匹配") {
        Color::Yellow
    } else if part.contains(" unknown") || part.contains(" 未知") {
        Color::DarkGray
    } else {
        Color::White
    }
}

fn key_value_color(key: &str) -> Color {
    match key {
        "fix" | "修复" => Color::Green,
        "blocked by" | "requires sudo" | "note" | "阻塞依赖" | "需要 sudo：是" | "备注" => {
            Color::Yellow
        }
        "command" | "instruction" | "Target channel" | "Target platform" | "Policy source" => {
            Color::Cyan
        }
        "命令" | "操作说明" | "目标通道" | "目标平台" | "来源" => Color::Cyan,
        "file" | "文件" => Color::Magenta,
        "path" | "active_path" | "PATH candidates" | "candidates" | "路径" | "PATH 候选"
        | "候选项" => Color::Blue,
        "current" | "当前" => Color::Green,
        "required" | "要求" => Color::Yellow,
        "manager" | "管理器" => Color::Magenta,
        "Policy auto-fix" | "Output" | "策略自动修复" | "输出" => Color::Cyan,
        _ => Color::DarkGray,
    }
}

fn sync_plan_output(plan: SyncPlan, messages: Messages) -> ActionOutput {
    let mut lines = vec![format!(
        "{}{}",
        messages.text(Text::SyncPlan),
        messages.dry_run_suffix(plan.dry_run)
    )];
    if let Some(channel) = &plan.policy_channel {
        lines.push(format!("{}: {channel}", messages.text(Text::TargetChannel)));
    }
    if let Some(platform) = &plan.platform {
        lines.push(format!(
            "{}: {platform}",
            messages.text(Text::TargetPlatform)
        ));
    }
    if plan.auto_fix {
        lines.push(messages.text(Text::PolicyAutoFixEnabled).to_string());
    }
    lines.push(String::new());

    let ready_steps = plan
        .graph
        .ready
        .iter()
        .filter_map(|id| plan.steps.iter().find(|step| step.id == *id))
        .collect::<Vec<_>>();
    if !ready_steps.is_empty() {
        lines.push(messages.text(Text::Ready).to_string());
        for step in ready_steps {
            push_sync_step_lines(&mut lines, step, messages);
        }
        lines.push(String::new());
    }

    if !plan.graph.blocked.is_empty() {
        lines.push(messages.text(Text::Blocked).to_string());
        for blocked in &plan.graph.blocked {
            if let Some(step) = plan.steps.iter().find(|step| step.id == blocked.id) {
                push_sync_step_lines(&mut lines, step, messages);
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
        lines.push(messages.text(Text::Verify).to_string());
        for step in verify_steps {
            push_sync_step_lines(&mut lines, step, messages);
        }
    }

    ActionOutput::ok(messages.text(Text::PreviewSync), lines)
}

fn push_sync_step_lines(lines: &mut Vec<String>, step: &crate::sync::SyncStep, messages: Messages) {
    lines.push(format!(
        "- [{}] {} - {}",
        kind_label(&step.kind, messages),
        step.target,
        step.reason
    ));
    if !step.blocked_by.is_empty() {
        lines.push(format!(
            "   {}: {}",
            messages.text(Text::BlockedBy),
            step.blocked_by.join(", ")
        ));
    }
    if let Some(command) = &step.command {
        let label = if step.manual {
            messages.text(Text::Instruction)
        } else {
            messages.text(Text::Command)
        };
        lines.push(format!("   {label}: {command}"));
    }
    if let Some(file) = &step.file {
        lines.push(format!(
            "   {}: {}",
            messages.text(Text::File),
            file.display()
        ));
    }
    if let Some(snippet) = &step.snippet {
        lines.push(format!(
            "   {}: {}",
            messages.text(Text::Snippet),
            snippet.replace('\n', "\n            ")
        ));
    }
    if step.requires_sudo {
        lines.push(format!("   {}", messages.text(Text::RequiresSudoYes)));
    }
}

fn sync_execution_output(execution: SyncExecution, messages: Messages) -> ActionOutput {
    let mut lines = vec![messages.text(Text::SyncExecution).to_string()];
    if let Some(channel) = &execution.policy_channel {
        lines.push(format!("{}: {channel}", messages.text(Text::TargetChannel)));
    }
    if let Some(platform) = &execution.platform {
        lines.push(format!(
            "{}: {platform}",
            messages.text(Text::TargetPlatform)
        ));
    }
    if execution.auto_fix {
        lines.push(messages.text(Text::PolicyAutoFixEnabled).to_string());
    }
    lines.push(String::new());

    for step in &execution.steps {
        lines.push(format!(
            "- [{}] {}: {}",
            execution_status_label(&step.status, messages),
            step.target,
            step.detail
        ));
        if !step.blocked_by.is_empty() {
            lines.push(format!(
                "  {}: {}",
                messages.text(Text::BlockedBy),
                step.blocked_by.join(", ")
            ));
        }
        if let Some(command) = &step.command {
            let label = if step.manual {
                messages.text(Text::Instruction)
            } else {
                messages.text(Text::Command)
            };
            lines.push(format!("  {label}: {command}"));
        }
        if let Some(file) = &step.file {
            lines.push(format!(
                "  {}: {}",
                messages.text(Text::File),
                file.display()
            ));
        }
    }

    lines.push(String::new());
    if execution.succeeded {
        lines.push(format!(
            "{}: {}",
            messages.text(Text::Result),
            messages.text(Text::ResultEnvironmentMatchesPolicy)
        ));
    } else {
        lines.push(format!(
            "{}: {}",
            messages.text(Text::Result),
            messages.text(Text::ResultSyncStopped)
        ));
    }

    ActionOutput {
        title: messages.text(Text::ApplySync).to_string(),
        lines,
        ok: execution.succeeded,
        mark_saved: false,
    }
}

fn status_label(status: &Status, messages: Messages) -> &'static str {
    match status {
        Status::Ok => messages.label(Label::Ok),
        Status::Missing => messages.label(Label::Missing),
        Status::Mismatch => messages.label(Label::Mismatch),
        Status::Unknown => messages.label(Label::Unknown),
    }
}

fn issue_severity_label(severity: IssueSeverity, messages: Messages) -> &'static str {
    match severity {
        IssueSeverity::Error => messages.label(Label::Error),
        IssueSeverity::Warning => messages.label(Label::Warning),
        IssueSeverity::Info => messages.label(Label::Info),
    }
}

fn kind_label(kind: &SyncStepKind, messages: Messages) -> &'static str {
    match kind {
        SyncStepKind::Install => messages.label(Label::Install),
        SyncStepKind::Align => messages.label(Label::Align),
        SyncStepKind::Configure => messages.label(Label::Configure),
        SyncStepKind::Cleanup => messages.label(Label::Cleanup),
        SyncStepKind::Verify => messages.label(Label::Verify),
        SyncStepKind::Info => messages.label(Label::Info),
    }
}

fn execution_status_label(status: &SyncStepExecutionStatus, messages: Messages) -> &'static str {
    match status {
        SyncStepExecutionStatus::Applied => messages.label(Label::Applied),
        SyncStepExecutionStatus::Unchanged => messages.label(Label::Unchanged),
        SyncStepExecutionStatus::Skipped => messages.label(Label::Skipped),
        SyncStepExecutionStatus::Failed => messages.label(Label::Failed),
        SyncStepExecutionStatus::Verified => messages.label(Label::Verified),
    }
}

fn tool_field_rows(draft: &InitDraft, tool: &'static str, messages: Messages) -> Vec<FieldRow> {
    let mut rows = vec![FieldRow {
        label: messages.label(Label::Enabled).to_string(),
        value: if tool_enabled(draft, tool) {
            messages.label(Label::Enabled).to_string()
        } else {
            messages.label(Label::Disabled).to_string()
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
                    label: tui_text(messages, TuiText::Version).to_string(),
                    value: node.version.clone(),
                    target: FieldTarget::NodeVersion,
                });
                rows.push(FieldRow {
                    label: messages.text(Text::Manager).to_string(),
                    value: node.manager.clone(),
                    target: FieldTarget::NodeManager,
                });
                rows.push(FieldRow {
                    label: tui_text(messages, TuiText::NodePackageManagers).to_string(),
                    value: list_value(Some(&node.package_managers)),
                    target: FieldTarget::NodePackageManagers,
                });
            }
        }
        "go" => {
            if let Some(go) = &draft.go {
                rows.push(FieldRow {
                    label: tui_text(messages, TuiText::Version).to_string(),
                    value: go.version.clone(),
                    target: FieldTarget::GoVersion,
                });
                rows.push(FieldRow {
                    label: messages.text(Text::Manager).to_string(),
                    value: go.manager.clone(),
                    target: FieldTarget::GoManager,
                });
                rows.push(FieldRow {
                    label: messages.text(Text::Source).to_string(),
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
                    label: messages.text(Text::Channel).to_string(),
                    value: rust.channel.clone(),
                    target: FieldTarget::RustChannel,
                });
            }
        }
        simple => {
            if let Some(tool) = simple_tool(draft, simple) {
                rows.push(FieldRow {
                    label: messages.text(Text::Manager).to_string(),
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
                node.package_managers = normalize_node_package_managers(parse_list(value));
            }
            sync_node_package_manager_tools(draft);
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

fn normalize_node_package_manager_state(draft: &mut InitDraft) {
    if let Some(node) = &mut draft.node {
        node.package_managers =
            normalize_node_package_managers(std::mem::take(&mut node.package_managers));
    }
    sync_node_package_manager_tools(draft);
}

fn set_tool_enabled(draft: &mut InitDraft, tool: &str, enabled: bool) {
    match tool {
        "fnm" => set_simple_tool(&mut draft.fnm, enabled, default_manager("fnm")),
        "nvm" => set_simple_tool(&mut draft.nvm, enabled, "standalone"),
        "node" => {
            if enabled {
                draft.node.get_or_insert_with(default_node_draft);
                normalize_node_package_manager_state(draft);
            } else {
                draft.node = None;
                sync_node_package_manager_tools(draft);
            }
        }
        "npm" | "pnpm" | "yarn" | "bun" => {
            set_node_package_manager_tool_enabled(draft, tool, enabled);
        }
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

fn set_node_package_manager_tool_enabled(draft: &mut InitDraft, tool: &str, enabled: bool) {
    match tool {
        "npm" => set_simple_tool(&mut draft.npm, enabled, "npm"),
        "pnpm" => set_simple_tool(&mut draft.pnpm, enabled, "corepack"),
        "yarn" => set_simple_tool(&mut draft.yarn, enabled, "corepack"),
        "bun" => set_simple_tool(&mut draft.bun, enabled, default_manager("bun")),
        _ => return,
    }
    set_node_package_manager_enabled(draft, tool, enabled);
}

fn set_node_package_manager_enabled(draft: &mut InitDraft, package_manager: &str, enabled: bool) {
    if enabled {
        if draft.node.is_none() {
            let mut node = default_node_draft();
            node.package_managers.clear();
            draft.node = Some(node);
        }
        if let Some(node) = &mut draft.node
            && !node
                .package_managers
                .iter()
                .any(|manager| manager == package_manager)
        {
            node.package_managers.push(package_manager.to_string());
            node.package_managers =
                normalize_node_package_managers(std::mem::take(&mut node.package_managers));
        }
    } else if let Some(node) = &mut draft.node {
        node.package_managers
            .retain(|manager| manager != package_manager);
    }
}

fn sync_node_package_manager_tools(draft: &mut InitDraft) {
    let package_managers = draft
        .node
        .as_ref()
        .map(|node| node.package_managers.clone())
        .unwrap_or_default();
    for tool in NODE_PACKAGE_MANAGER_TOOLS {
        let enabled = package_managers.iter().any(|manager| manager == tool);
        match *tool {
            "npm" => set_simple_tool(&mut draft.npm, enabled, "npm"),
            "pnpm" => set_simple_tool(&mut draft.pnpm, enabled, "corepack"),
            "yarn" => set_simple_tool(&mut draft.yarn, enabled, "corepack"),
            "bun" => set_simple_tool(&mut draft.bun, enabled, default_manager("bun")),
            _ => {}
        }
    }
}

fn normalize_node_package_managers(package_managers: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for known in NODE_PACKAGE_MANAGER_TOOLS {
        if package_managers
            .iter()
            .any(|package_manager| package_manager.eq_ignore_ascii_case(known))
        {
            normalized.push((*known).to_string());
        }
    }
    for package_manager in package_managers {
        if NODE_PACKAGE_MANAGER_TOOLS
            .iter()
            .any(|known| package_manager.eq_ignore_ascii_case(known))
            || normalized.iter().any(|known| known == &package_manager)
        {
            continue;
        }
        normalized.push(package_manager);
    }
    normalized
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

fn tool_summary(draft: &InitDraft, tool: &str, messages: Messages) -> String {
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
        "npm" | "pnpm" | "yarn" | "bun" => simple_tool(draft, tool)
            .map(|tool| match messages.language() {
                Language::En => format!("node package via {}", tool.manager),
                Language::Zh => format!("node package via {}", tool.manager),
            })
            .unwrap_or_default(),
        simple => simple_tool(draft, simple)
            .map(|tool| format!("via {}", tool.manager))
            .unwrap_or_default(),
    }
}

fn enabled_label(enabled: bool, messages: Messages) -> &'static str {
    if enabled {
        messages.label(Label::Enabled)
    } else {
        messages.label(Label::Disabled)
    }
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "on" | "enabled" | "已启用"
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
        sync::mpsc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Color;
    use ratatui::text::Line;

    use crate::doctor::{
        DoctorReport, Issue, IssueEvidence, IssueKind, IssueSeverity, Status, ToolStatus,
    };
    use crate::i18n::{Language, Messages};
    use crate::init::{
        CliDraft, InitDraft, NpmDraft, PolicyDraft, SimpleToolDraft, render_init_document,
    };
    use crate::latest::{VersionCandidate, VersionCandidates};
    use crate::sync::{SyncBlockedStep, SyncPlan, SyncPlanGraph, SyncStep, SyncStepKind};

    use super::{
        ActionOutput, AppExit, FieldTarget, Focus, InitTuiApp, InitTuiOptions, MenuEntry,
        VersionPickerState, action_field_rows, action_menu_index, action_output_line_count,
        apply_field_edit, default_node_draft, doctor_output, field_rows, menu_entries, move_index,
        parse_list, set_tool_enabled, styled_action_output_line, sync_plan_output, tool_enabled,
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
        assert!(tool_enabled(&draft, "npm"));
        assert!(tool_enabled(&draft, "pnpm"));
        assert!(tool_enabled(&draft, "yarn"));
        assert!(tool_enabled(&draft, "bun"));
        assert_eq!(draft.node.as_ref().unwrap().manager, "fnm");
        assert_eq!(
            draft.node.as_ref().unwrap().package_managers,
            vec![
                "npm".to_string(),
                "pnpm".to_string(),
                "yarn".to_string(),
                "bun".to_string()
            ]
        );
        assert_eq!(draft.nvm.as_ref().unwrap().manager, "standalone");
        assert_eq!(draft.bun.as_ref().unwrap().manager, "brew");
        assert_eq!(draft.go.as_ref().unwrap().source, "brew");

        let document = render_init_document(&draft);
        assert!(document.content.contains("[tools.nvm]"));
        assert!(document.content.contains("manager = \"standalone\""));

        set_tool_enabled(&mut draft, "node", false);
        assert!(!tool_enabled(&draft, "node"));
        assert!(!tool_enabled(&draft, "npm"));
        assert!(!tool_enabled(&draft, "pnpm"));
        assert!(!tool_enabled(&draft, "yarn"));
        assert!(!tool_enabled(&draft, "bun"));
    }

    #[test]
    fn node_package_manager_tool_toggles_sync_node_workflow() {
        let mut draft = empty_draft();
        draft.node = Some(default_node_draft());
        set_tool_enabled(&mut draft, "node", true);

        set_tool_enabled(&mut draft, "yarn", false);

        assert!(!tool_enabled(&draft, "yarn"));
        assert_eq!(
            draft.node.as_ref().unwrap().package_managers,
            vec!["npm".to_string(), "pnpm".to_string(), "bun".to_string()]
        );
        let document = render_init_document(&draft);
        assert!(!document.content.contains("[tools.yarn]"));
        assert!(
            document
                .content
                .contains("package_managers = [\"npm\", \"pnpm\", \"bun\"]")
        );

        set_tool_enabled(&mut draft, "yarn", true);

        assert!(tool_enabled(&draft, "yarn"));
        assert_eq!(
            draft.node.as_ref().unwrap().package_managers,
            vec![
                "npm".to_string(),
                "pnpm".to_string(),
                "yarn".to_string(),
                "bun".to_string()
            ]
        );
    }

    #[test]
    fn editing_node_package_managers_syncs_tool_toggles() {
        let mut draft = empty_draft();
        set_tool_enabled(&mut draft, "node", true);

        apply_field_edit(
            &mut draft,
            &FieldTarget::NodePackageManagers,
            "npm, pnpm, bun",
        );

        assert!(tool_enabled(&draft, "npm"));
        assert!(tool_enabled(&draft, "pnpm"));
        assert!(!tool_enabled(&draft, "yarn"));
        assert!(tool_enabled(&draft, "bun"));

        apply_field_edit(
            &mut draft,
            &FieldTarget::NodePackageManagers,
            "npm, pnpm, yarn, bun",
        );

        assert!(tool_enabled(&draft, "yarn"));
    }

    #[test]
    fn tui_open_normalizes_node_package_manager_tool_sections() {
        let mut draft = empty_draft();
        let mut node = default_node_draft();
        node.package_managers = vec!["npm".to_string(), "pnpm".to_string(), "bun".to_string()];
        draft.node = Some(node);
        draft.yarn = Some(SimpleToolDraft {
            manager: "corepack".to_string(),
        });

        let app = InitTuiApp::new(draft);

        assert!(tool_enabled(&app.draft, "npm"));
        assert!(tool_enabled(&app.draft, "pnpm"));
        assert!(!tool_enabled(&app.draft, "yarn"));
        assert!(tool_enabled(&app.draft, "bun"));
        assert!(app.draft.yarn.is_none());
        let document = render_init_document(&app.draft);
        assert!(!document.content.contains("[tools.yarn]"));
        assert!(
            document
                .content
                .contains("package_managers = [\"npm\", \"pnpm\", \"bun\"]")
        );
    }

    #[test]
    fn tui_open_creates_tool_sections_from_node_package_managers() {
        let mut draft = empty_draft();
        let mut node = default_node_draft();
        node.package_managers = vec!["YARN".to_string()];
        draft.node = Some(node);

        let app = InitTuiApp::new(draft);

        assert_eq!(
            app.draft.node.as_ref().unwrap().package_managers,
            vec!["yarn".to_string()]
        );
        assert!(tool_enabled(&app.draft, "yarn"));
        assert_eq!(app.draft.yarn.as_ref().unwrap().manager, "corepack");
        let document = render_init_document(&app.draft);
        assert!(document.content.contains("[tools.yarn]"));
        assert!(document.content.contains("package_managers = [\"yarn\"]"));
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

        let output = sync_plan_output(plan, Messages::english());
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
    fn package_sections_use_field_labels_instead_of_repeating_section_titles() {
        let mut draft = empty_draft();
        draft.cli = Some(CliDraft {
            manager: "brew".to_string(),
            packages: vec!["gh".to_string()],
        });
        draft.npm_config = Some(NpmDraft {
            global_packages: vec!["wrangler".to_string()],
        });
        let zh_options = InitTuiOptions {
            output: PathBuf::from("devkit.toml"),
            force: false,
            stdout: false,
            language: Language::Zh,
        };

        let homebrew_rows = field_rows(&draft, MenuEntry::Homebrew, &zh_options);
        let npm_rows = field_rows(&draft, MenuEntry::Npm, &zh_options);

        assert_eq!(homebrew_rows[0].label, "包列表");
        assert_eq!(homebrew_rows[0].value, "gh");
        assert_eq!(npm_rows[0].label, "包列表");
        assert_eq!(npm_rows[0].value, "wrangler");
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
        assert_eq!(app.focus, Focus::SidePanel);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Fields);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Menu);
    }

    #[test]
    fn tab_focuses_preview_panel_and_arrows_scroll() {
        let mut app = InitTuiApp::new(empty_draft());
        app.focus = Focus::Menu;
        app.menu_index = 0;

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(app.focus, Focus::SidePanel);
        assert!(!app.preview_expanded);
        assert!(!app.action_expanded);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(app.preview_scroll > 0);
    }

    #[test]
    fn tab_focuses_action_output_and_arrows_scroll() {
        let mut app = InitTuiApp::new(empty_draft());
        app.menu_index = action_menu_index();
        app.focus = Focus::Fields;
        app.action_output = Some(ActionOutput::ok(
            "Check environment",
            (0..30).map(|index| format!("line {index}")).collect(),
        ));

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(app.focus, Focus::SidePanel);
        assert!(!app.action_expanded);
        assert!(!app.preview_expanded);
        assert_eq!(app.menu_index, action_menu_index());

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(app.action_scroll > 0);
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
        assert_eq!(app.focus, Focus::SidePanel);

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
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
    fn actions_list_excludes_finish_action() {
        let labels = action_field_rows(&InitTuiOptions {
            output: PathBuf::from("devkit.toml"),
            force: false,
            stdout: false,
            language: Language::En,
        })
        .into_iter()
        .map(|row| row.label)
        .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec!["save config", "run check", "preview sync", "apply sync"]
        );
    }

    #[test]
    fn actions_list_uses_selected_language() {
        let labels = action_field_rows(&InitTuiOptions {
            output: PathBuf::from("devkit.toml"),
            force: false,
            stdout: false,
            language: Language::Zh,
        })
        .into_iter()
        .map(|row| row.label)
        .collect::<Vec<_>>();

        assert_eq!(labels, vec!["保存配置", "运行检查", "预览同步", "执行同步"]);
    }

    #[test]
    fn language_key_toggles_runtime_tui_language() {
        let mut app = InitTuiApp::new(empty_draft());
        app.menu_index = action_menu_index();

        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));

        assert_eq!(app.options.language, Language::Zh);
        assert_eq!(app.status, "已切换为中文");
        let labels = action_field_rows(&app.options)
            .into_iter()
            .map(|row| row.label)
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["保存配置", "运行检查", "预览同步", "执行同步"]);

        app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE));

        assert_eq!(app.options.language, Language::En);
        assert_eq!(app.status, "Language switched to English");
    }

    #[test]
    fn language_key_works_in_preview_but_not_inside_edit_input() {
        let mut app = InitTuiApp::new(empty_draft());
        app.open_preview();

        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));

        assert_eq!(app.options.language, Language::Zh);
        assert!(app.preview_expanded);

        app.close_preview();
        app.focus = Focus::Fields;
        app.start_edit_or_focus_fields();
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));

        assert_eq!(app.options.language, Language::Zh);
        assert!(app.edit.as_ref().unwrap().buffer.ends_with('l'));
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
    fn action_task_updates_visible_progress_before_completion() {
        let mut app = InitTuiApp::new(empty_draft());
        let (release_sender, release_receiver) = mpsc::channel();

        app.start_action_task("Test action", move |_draft, _options, progress| {
            progress.update_detail("Building plan", "checking dependencies");
            release_receiver.recv().unwrap();
            Ok(ActionOutput::ok("Done", vec!["finished".to_string()]))
        });

        wait_until(|| {
            app.poll_action_task();
            app.action_task
                .as_ref()
                .is_some_and(|task| task.progress.label == "Building plan")
        });

        let task = app.action_task.as_ref().unwrap();
        assert_eq!(
            task.progress.detail.as_deref(),
            Some("checking dependencies")
        );
        assert_eq!(app.status, "Test action: Building plan");

        release_sender.send(()).unwrap();
        wait_until(|| {
            app.poll_action_task();
            app.action_task.is_none()
        });

        assert_eq!(app.status, "Test action finished");
        assert!(app.action_output.is_some());
    }

    #[test]
    fn doctor_output_formats_path_candidates_as_evidence() {
        let report = DoctorReport {
            tools: vec![ToolStatus {
                name: "python".to_string(),
                command: "python3".to_string(),
                path: Some(PathBuf::from("/opt/homebrew/bin/python3")),
                path_candidates: vec![
                    PathBuf::from("/opt/homebrew/bin/python3"),
                    PathBuf::from("/usr/bin/python3"),
                ],
                current: Some("3.14.4".to_string()),
                required: Some("latest".to_string()),
                manager: Some("brew".to_string()),
                status: Status::Ok,
                note: None,
            }],
            issues: vec![Issue {
                kind: IssueKind::PathConflict,
                severity: IssueSeverity::Warning,
                path: Some(PathBuf::from("/opt/homebrew/bin/python3")),
                message: "python resolves to the first of 2 PATH candidates".to_string(),
                evidence: vec![
                    IssueEvidence {
                        key: "active_path".to_string(),
                        value: "/opt/homebrew/bin/python3".to_string(),
                    },
                    IssueEvidence {
                        key: "candidate_count".to_string(),
                        value: "2".to_string(),
                    },
                    IssueEvidence {
                        key: "candidates".to_string(),
                        value: "/opt/homebrew/bin/python3; /usr/bin/python3".to_string(),
                    },
                ],
                fix: Some(
                    "inspect PATH ordering; candidates: /opt/homebrew/bin/python3; /usr/bin/python3"
                        .to_string(),
                ),
            }],
        };

        let output = doctor_output(
            report,
            &InitTuiOptions {
                output: PathBuf::from("devkit.toml"),
                force: false,
                stdout: true,
                language: Language::En,
            },
        );
        let text = output.lines.join("\n");

        assert!(text.contains("- python [ok]"));
        assert!(text.contains("  PATH candidates: 2"));
        assert!(text.contains("    1. /opt/homebrew/bin/python3 (active)"));
        assert!(text.contains("  candidates:"));
        assert!(text.contains("    2. /usr/bin/python3"));
        assert!(text.contains("  fix: inspect PATH ordering"));
        assert!(!text.contains("fix: inspect PATH ordering; candidates:"));
    }

    #[test]
    fn action_output_lines_style_structured_statuses() {
        let tool = styled_action_output_line("- python [ok]");
        assert!(span_has_color(&tool, "ok", Color::Green));

        let issue = styled_action_output_line("- [warning] python resolves to the first candidate");
        assert!(span_has_color(&issue, "warning", Color::Yellow));

        let fix = styled_action_output_line("  fix: brew install poetry");
        assert!(span_has_color(&fix, "fix", Color::Green));

        let candidate = styled_action_output_line("    1. /opt/homebrew/bin/python3 (active)");
        assert!(span_has_color(&candidate, " (active)", Color::Green));

        let lines = vec!["one\ntwo".to_string(), String::new()];
        assert_eq!(action_output_line_count(&lines), 3);
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
                language: Language::En,
            },
        );

        let exit = app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));

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
                language: Language::En,
            },
        );

        let exit = app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));

        assert!(matches!(exit, Some(AppExit::Continue)));
        assert!(!path.exists());
    }

    fn span_has_color(line: &Line<'_>, content: &str, color: Color) -> bool {
        line.spans
            .iter()
            .any(|span| span.content.as_ref() == content && span.style.fg == Some(color))
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

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(2) {
            if condition() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("condition did not become true before timeout");
    }
}
