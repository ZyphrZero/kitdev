use clap::ValueEnum;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Language {
    #[value(name = "en", alias = "en-US", alias = "en_US")]
    #[default]
    En,
    #[value(name = "zh", alias = "zh-CN", alias = "zh_CN", alias = "cn")]
    Zh,
}

#[derive(Debug, Clone, Copy)]
pub struct Messages {
    language: Language,
}

#[derive(Debug, Clone, Copy)]
pub enum Text {
    ConfigValidation,
    ConfigValidationOk,
    ErrorsFound,
    WarningsOnly,
    ConfigExplain,
    Platform,
    AppliedOverrides,
    Entries,
    None,
    Tool,
    Current,
    Required,
    Status,
    Path,
    Issues,
    Fix,
    UpgradePlan,
    WithLatestChecks,
    Source,
    Command,
    CleanupPlan,
    NoLegacyLeftovers,
    RequiresSudoYes,
    Wrote,
    Overwrote,
    Cancelled,
    InstallPlan,
    InstallExecution,
    TargetTool,
    TargetChannel,
    TargetPlatform,
    PolicyAutoFixEnabled,
    Ready,
    Blocked,
    Verify,
    SyncPlan,
    SyncExecution,
    Result,
    ResultInstallCompleted,
    ResultInstallNotNeeded,
    ResultInstallFailed,
    ResultEnvironmentMatchesPolicy,
    ResultSyncStopped,
    Detected,
    DetectedNotVisibleInPath,
    Instruction,
    File,
    Snippet,
    BlockedBy,
    PathCandidates,
    PolicyBuilder,
    Running,
    Unsaved,
    Saved,
    Channel,
    EnabledTools,
    Output,
    Sections,
    Actions,
    Preview,
    Edit,
    Confirm,
    Versions,
    Policy,
    HomebrewPackages,
    NpmGlobals,
    PackageList,
    SaveConfig,
    RunCheck,
    PreviewSync,
    ApplySync,
    CheckEnvironment,
    GeneratedToml,
    SavedConfig,
    DoctorReport,
    Tools,
    Summary,
    Manager,
    Note,
    Candidates,
    Active,
    Finish,
    Error,
}

#[derive(Debug, Clone, Copy)]
pub enum Label {
    Ok,
    Missing,
    Mismatch,
    Unknown,
    Error,
    Warning,
    Info,
    Install,
    Align,
    Configure,
    Cleanup,
    Verify,
    Applied,
    Unchanged,
    Skipped,
    Failed,
    Verified,
    Enabled,
    Disabled,
}

impl Language {
    pub fn detect(explicit: Option<Self>) -> Self {
        explicit.unwrap_or_else(Self::from_env)
    }

    pub fn from_env() -> Self {
        ["DEVKIT_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok())
            .find_map(|value| Self::from_locale(&value))
            .unwrap_or(Self::En)
    }

    pub fn from_locale(locale: &str) -> Option<Self> {
        let normalized = locale.trim().to_ascii_lowercase().replace('_', "-");
        if normalized.is_empty() || normalized == "c" || normalized == "posix" {
            return None;
        }
        if normalized.starts_with("zh") || normalized == "cn" {
            Some(Self::Zh)
        } else if normalized.starts_with("en") {
            Some(Self::En)
        } else {
            None
        }
    }
}

impl Messages {
    pub fn new(language: Language) -> Self {
        Self { language }
    }

    pub fn english() -> Self {
        Self::new(Language::En)
    }

    pub fn language(self) -> Language {
        self.language
    }

    pub fn text(self, text: Text) -> &'static str {
        match self.language {
            Language::En => english_text(text),
            Language::Zh => chinese_text(text),
        }
    }

    pub fn label(self, label: Label) -> &'static str {
        match self.language {
            Language::En => english_label(label),
            Language::Zh => chinese_label(label),
        }
    }

    pub fn dry_run_suffix(self, dry_run: bool) -> &'static str {
        if !dry_run {
            return "";
        }
        match self.language {
            Language::En => " (dry-run)",
            Language::Zh => "（预览）",
        }
    }
}

impl Default for Messages {
    fn default() -> Self {
        Self::english()
    }
}

fn english_text(text: Text) -> &'static str {
    match text {
        Text::ConfigValidation => "Config validation",
        Text::ConfigValidationOk => "Config validation: ok",
        Text::ErrorsFound => "errors found",
        Text::WarningsOnly => "warnings only",
        Text::ConfigExplain => "Config explain",
        Text::Platform => "platform",
        Text::AppliedOverrides => "applied overrides",
        Text::Entries => "entries",
        Text::None => "none",
        Text::Tool => "Tool",
        Text::Current => "Current",
        Text::Required => "Required",
        Text::Status => "Status",
        Text::Path => "path",
        Text::Issues => "Issues",
        Text::Fix => "fix",
        Text::UpgradePlan => "Upgrade plan",
        Text::WithLatestChecks => " with latest checks",
        Text::Source => "source",
        Text::Command => "command",
        Text::CleanupPlan => "Cleanup plan",
        Text::NoLegacyLeftovers => "no known legacy toolchain leftovers found",
        Text::RequiresSudoYes => "requires sudo: yes",
        Text::Wrote => "Wrote",
        Text::Overwrote => "Overwrote",
        Text::Cancelled => "Cancelled",
        Text::InstallPlan => "Install plan",
        Text::InstallExecution => "Install execution",
        Text::TargetTool => "target tool",
        Text::TargetChannel => "target channel",
        Text::TargetPlatform => "target platform",
        Text::PolicyAutoFixEnabled => "policy auto-fix: enabled",
        Text::Ready => "Ready",
        Text::Blocked => "Blocked",
        Text::Verify => "Verify",
        Text::SyncPlan => "Sync plan",
        Text::SyncExecution => "Sync execution",
        Text::Result => "result",
        Text::ResultInstallCompleted => "install command completed",
        Text::ResultInstallNotNeeded => "no install command needed",
        Text::ResultInstallFailed => "install command failed",
        Text::ResultEnvironmentMatchesPolicy => "environment matches policy",
        Text::ResultSyncStopped => "sync stopped before reaching the configured policy",
        Text::Detected => "detected",
        Text::DetectedNotVisibleInPath => "is not visible in PATH yet",
        Text::Instruction => "instruction",
        Text::File => "file",
        Text::Snippet => "snippet",
        Text::BlockedBy => "blocked by",
        Text::PathCandidates => "PATH candidates",
        Text::PolicyBuilder => "policy builder",
        Text::Running => "running",
        Text::Unsaved => "unsaved",
        Text::Saved => "saved",
        Text::Channel => "channel",
        Text::EnabledTools => "enabled tools",
        Text::Output => "output",
        Text::Sections => "Sections",
        Text::Actions => "Actions",
        Text::Preview => "Preview",
        Text::Edit => "Edit",
        Text::Confirm => "Confirm",
        Text::Versions => "Versions",
        Text::Policy => "policy",
        Text::HomebrewPackages => "Homebrew packages",
        Text::NpmGlobals => "npm globals",
        Text::PackageList => "packages",
        Text::SaveConfig => "save config",
        Text::RunCheck => "run check",
        Text::PreviewSync => "preview sync",
        Text::ApplySync => "apply sync",
        Text::CheckEnvironment => "Check environment",
        Text::GeneratedToml => "Generated TOML",
        Text::SavedConfig => "Saved config",
        Text::DoctorReport => "Doctor report",
        Text::Tools => "Tools",
        Text::Summary => "Summary",
        Text::Manager => "manager",
        Text::Note => "note",
        Text::Candidates => "candidates",
        Text::Active => "active",
        Text::Finish => "Finish",
        Text::Error => "Error",
    }
}

fn chinese_text(text: Text) -> &'static str {
    match text {
        Text::ConfigValidation => "配置校验",
        Text::ConfigValidationOk => "配置校验：通过",
        Text::ErrorsFound => "发现错误",
        Text::WarningsOnly => "仅有警告",
        Text::ConfigExplain => "配置解释",
        Text::Platform => "平台",
        Text::AppliedOverrides => "已应用覆盖",
        Text::Entries => "条目",
        Text::None => "无",
        Text::Tool => "工具",
        Text::Current => "当前",
        Text::Required => "要求",
        Text::Status => "状态",
        Text::Path => "路径",
        Text::Issues => "问题",
        Text::Fix => "修复",
        Text::UpgradePlan => "升级计划",
        Text::WithLatestChecks => "（包含最新版本检查）",
        Text::Source => "来源",
        Text::Command => "命令",
        Text::CleanupPlan => "清理计划",
        Text::NoLegacyLeftovers => "未发现已知的旧工具链残留",
        Text::RequiresSudoYes => "需要 sudo：是",
        Text::Wrote => "已写入",
        Text::Overwrote => "已覆盖",
        Text::Cancelled => "已取消",
        Text::InstallPlan => "安装计划",
        Text::InstallExecution => "安装执行",
        Text::TargetTool => "目标工具",
        Text::TargetChannel => "目标通道",
        Text::TargetPlatform => "目标平台",
        Text::PolicyAutoFixEnabled => "策略自动修复：已启用",
        Text::Ready => "可执行",
        Text::Blocked => "被阻塞",
        Text::Verify => "验证",
        Text::SyncPlan => "同步计划",
        Text::SyncExecution => "同步执行",
        Text::Result => "结果",
        Text::ResultInstallCompleted => "安装命令已完成",
        Text::ResultInstallNotNeeded => "无需执行安装命令",
        Text::ResultInstallFailed => "安装命令失败",
        Text::ResultEnvironmentMatchesPolicy => "环境已匹配策略",
        Text::ResultSyncStopped => "同步在达到配置策略前停止",
        Text::Detected => "检测到",
        Text::DetectedNotVisibleInPath => "尚未出现在 PATH 中",
        Text::Instruction => "操作说明",
        Text::File => "文件",
        Text::Snippet => "片段",
        Text::BlockedBy => "阻塞依赖",
        Text::PathCandidates => "PATH 候选",
        Text::PolicyBuilder => "策略构建器",
        Text::Running => "运行中",
        Text::Unsaved => "未保存",
        Text::Saved => "已保存",
        Text::Channel => "通道",
        Text::EnabledTools => "已启用工具",
        Text::Output => "输出",
        Text::Sections => "区域",
        Text::Actions => "操作",
        Text::Preview => "预览",
        Text::Edit => "编辑",
        Text::Confirm => "确认",
        Text::Versions => "版本",
        Text::Policy => "策略",
        Text::HomebrewPackages => "Homebrew 包",
        Text::NpmGlobals => "npm 全局包",
        Text::PackageList => "包列表",
        Text::SaveConfig => "保存配置",
        Text::RunCheck => "运行检查",
        Text::PreviewSync => "预览同步",
        Text::ApplySync => "执行同步",
        Text::CheckEnvironment => "检查环境",
        Text::GeneratedToml => "生成的 TOML",
        Text::SavedConfig => "已保存配置",
        Text::DoctorReport => "环境检查报告",
        Text::Tools => "工具",
        Text::Summary => "摘要",
        Text::Manager => "管理器",
        Text::Note => "备注",
        Text::Candidates => "候选项",
        Text::Active => "当前生效",
        Text::Finish => "完成",
        Text::Error => "错误",
    }
}

fn english_label(label: Label) -> &'static str {
    match label {
        Label::Ok => "ok",
        Label::Missing => "missing",
        Label::Mismatch => "mismatch",
        Label::Unknown => "unknown",
        Label::Error => "error",
        Label::Warning => "warning",
        Label::Info => "info",
        Label::Install => "install",
        Label::Align => "align",
        Label::Configure => "configure",
        Label::Cleanup => "cleanup",
        Label::Verify => "verify",
        Label::Applied => "applied",
        Label::Unchanged => "unchanged",
        Label::Skipped => "skipped",
        Label::Failed => "failed",
        Label::Verified => "verified",
        Label::Enabled => "enabled",
        Label::Disabled => "disabled",
    }
}

fn chinese_label(label: Label) -> &'static str {
    match label {
        Label::Ok => "正常",
        Label::Missing => "缺失",
        Label::Mismatch => "不匹配",
        Label::Unknown => "未知",
        Label::Error => "错误",
        Label::Warning => "警告",
        Label::Info => "信息",
        Label::Install => "安装",
        Label::Align => "对齐",
        Label::Configure => "配置",
        Label::Cleanup => "清理",
        Label::Verify => "验证",
        Label::Applied => "已执行",
        Label::Unchanged => "未变化",
        Label::Skipped => "已跳过",
        Label::Failed => "失败",
        Label::Verified => "已验证",
        Label::Enabled => "已启用",
        Label::Disabled => "已禁用",
    }
}

#[cfg(test)]
mod tests {
    use super::{Language, Messages, Text};

    #[test]
    fn detects_chinese_locale_variants() {
        assert_eq!(Language::from_locale("zh_CN.UTF-8"), Some(Language::Zh));
        assert_eq!(Language::from_locale("zh-Hans"), Some(Language::Zh));
        assert_eq!(Language::from_locale("en_US.UTF-8"), Some(Language::En));
        assert_eq!(Language::from_locale("C"), None);
    }

    #[test]
    fn returns_translated_text() {
        assert_eq!(Messages::new(Language::Zh).text(Text::SyncPlan), "同步计划");
        assert_eq!(Messages::english().text(Text::SyncPlan), "Sync plan");
    }
}
