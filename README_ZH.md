# devkit

`devkit` 是一个面向个人和团队的开发工具链健康检查 CLI。它不试图替代 `mise`、`fnm`、`rustup`、`uv` 这类版本管理器，而是在它们之上进行检查、报告和修复计划生成。

English documentation: [README.md](README.md)

## 产品模型

`devkit` 是一个跨平台开发工具链策略控制器。它管理的是个人或团队希望机器具备的开发环境意图，而不是包管理器命令本身。具体命令只是把策略落实到当前平台时使用的执行细节。

核心模型是：

- `devkit.toml` 是策略文件：描述期望的工具链状态。
- `doctor` 是诊断引擎：检查本机环境，报告和策略之间的差异，并解释当前路径、管理器、版本、PATH 候选项等证据。
- `sync` 是策略执行计划器：把策略差异转换成带依赖关系的 `ready` / `blocked` 计划，只有在用户明确要求时才执行可自动化步骤。
- `init` 和 TUI 是策略编辑器：用于生成和调整一个策略文件，并在写入或执行前展示有效 TOML。

这意味着用户启用的是“开发能力”，不是内部 TOML 字段。一个工具配置段应该回答两个问题：

- 需要什么能力，例如 Node、Go、Rust、Python，或者一组 CLI 包。
- 这个能力在当前平台上如何管理，例如 `fnm`、`nvm`、`rustup`、`uv`、Homebrew、Windows Package Manager，或者 standalone installer。

Node 使用嵌套工作流模型：

- `[tools.node]` 启用 Node.js 运行时工作流。
- `tools.node.manager` 选择 Node 运行时管理器，例如 `fnm`、`nvm` 或平台包管理器。
- `tools.node.package_managers` 是已启用 Node 包管理器工作流的唯一事实来源。
- `[tools.npm]`、`[tools.pnpm]`、`[tools.yarn]`、`[tools.bun]` 描述每个已启用包管理器如何检查、安装或对齐。

例如：

```toml
[tools.node]
version = "24.x"
manager = "fnm"
package_managers = ["npm", "pnpm", "yarn", "bun"]

[tools.pnpm]
version = "latest"
manager = "corepack"

[tools.yarn]
version = "stable"
manager = "corepack"

[tools.bun]
version = "latest"
manager = "brew"
```

在 TUI 中，`node` 开关控制 Node 运行时工作流。`npm`、`pnpm`、`yarn`、`bun` 开关表示 Node 工作流下的包管理器能力，并且必须和 `node.package_managers` 保持同步。关闭其中一个包管理器时，会从 `node.package_managers` 中移除它，并从生成的策略里移除对应工具段。重新打开时，会把它加回 `node.package_managers` 并创建对应工具段。关闭 Node 时，草稿中的包管理器工作流也会关闭，因为它们不再有所属的运行时工作流。

`devkit` 不应该因为某个工具已经安装在机器上，就默认管理或安装它。策略才是意图来源。未配置的工具可以在有帮助时作为环境上下文报告，但 `doctor` 和 `sync` 应该聚焦于已配置工具，以及已配置工具所需的依赖。

## MVP 命令

```bash
cargo run -- doctor
cargo run -- doctor -j
cargo run -- doctor -c examples/devkit.toml
cargo run -- init -p
cargo run -- init -i -p
cargo run -- tui
cargo run -- tui -p
cargo run -- init --output /tmp/devkit.toml
cargo run -- config validate -c examples/devkit.toml
cargo run -- config explain -c examples/devkit.toml
cargo run -- install bun
cargo run -- install bun -n
cargo run -- sync -c examples/devkit.toml
cargo run -- sync --apply -c examples/devkit.toml
cargo run -- upgrade -n -c examples/devkit.toml
cargo run -- upgrade -n --offline -c examples/devkit.toml
cargo run -- cleanup --dry-run
```

## 范围

`devkit` 检查 macOS、Linux 和 Windows 上常见的开发工具：

- Homebrew / fnm / nvm / Node / npm / pnpm / Bun / Wrangler
- Yarn / Deno / Python / Poetry / Ruby
- uv / Rust / Cargo / Go
- 策略需要的平台包管理器，例如 Homebrew 和 Windows Package Manager
- 遗留残留项，例如未配置 nvm 时的 `/usr/local/go`、`/usr/local/lib/node_modules`、`~/.nvm`，以及 Windows 上的 npm/nvm 路径
- PATH 中解析到多个可执行文件的候选项数量

平台默认值保持保守：

- macOS 保留以 Homebrew 为主的默认值。
- Linux 偏向 standalone installer、`rustup`、官方 Go，以及用于 Python 运行时的 `uv`；当 CLI 包没有配置管理器时，使用当前机器上最先检测到的 `apt`、`dnf`、`pacman`、`zypper`、`apk` 或 Linuxbrew。
- Windows 在有稳定包 ID 的场景优先使用 `winget`，standalone 工具使用 PowerShell installer，Rust 使用 `rustup`，Python 运行时使用 `uv`；当 CLI 包没有配置管理器时，使用当前机器上最先检测到的 `winget`、`scoop` 或 `choco`。

## Bootstrap 计划

日常命令只保留一条正式路径：

- `devkit doctor`：检查本机环境
- `devkit tui`：打开可视化策略编辑器
- `devkit init -i -p`：交互式打印 starter config
- `devkit config validate`：校验有效的单文件策略
- `devkit config explain`：显示 base 和 platform override 中哪些值被应用
- `devkit install bun`：安装单个工具
- `devkit sync`：预览策略修复计划
- `devkit sync --apply`：应用策略修复计划
- `devkit upgrade -n`：预览升级

`install <tool>` 是安装单个工具的正式路径。它默认执行，并在存在 `devkit.toml` 时读取配置；`-n` / `--dry-run` 会先打印精确命令：

- `devkit install bun` 使用配置或平台默认管理器安装 Bun。
- `devkit install deno` 在 macOS 上使用 Homebrew，在 Linux 上使用 standalone installer，在 Windows 上使用 `winget`。
- `devkit install node -v 24` 通过 `fnm` 安装并选择 Node。
- `devkit install node -m nvm -v 24` 通过 `nvm` 安装并选择 Node。
- `devkit install python -m uv -v 3.13` 通过 `uv` 安装 Python 运行时。
- `devkit install gh -c examples/devkit.toml` 安装或检查策略中列出的 CLI 包。
- `devkit install gh -m brew` 在没有策略条目时安装显式 Homebrew CLI 包。
- `devkit install GitHub.cli -m winget` 安装显式 Windows Package Manager 包 ID。
- `devkit install gh -c team.toml` 可以在策略列出包但省略 `manager` 时，从当前机器推断 `[tools.cli]` 管理器。

`sync` 会把当前机器状态和 `devkit.toml` 策略转换成 bootstrap 或修复计划。`sync --apply` 会应用 install、align 和受管理 shell 配置步骤，然后用 `doctor` 验证结果。

- 安装前置管理器，例如 Homebrew、Windows Package Manager、`fnm`、`nvm`、`rustup` 和 `uv`。
- 将 Node、Go、Rust 等受管理运行时对齐到策略。
- 将 dry-run 计划分组为 `ready` 和 `blocked` 图结构，避免依赖未满足时执行后续步骤。
- 为 `fnm` 和 Go PATH 设置规划平台合适的 shell snippet。
- 当 Node 配置为 `manager = "nvm"` 时规划 `nvm` shell 初始化。
- 基于实际环境报告重复 PATH 候选项，而不是依赖平台特定的可疑路径列表。
- 在尝试依赖命令前显示 blocker，例如 `tool:fnm`、`tool:npm` 或 `tool:winget`。
- 包含已配置的 Homebrew、npm、winget 和显式 Linux 包管理器 CLI 包。
- 使用 `devkit` marker 幂等执行受管理 shell snippet。
- 即使在 `sync --apply` 下，cleanup 步骤也保持手动。
- 最后执行 `doctor` 验证步骤。

## 配置生成

`init` 会把当前机器转换成 starter `devkit.toml`。默认模式是确定性的；`--interactive` 会打开可视化 TUI，可在写入前裁剪工具、编辑细节并查看实时 TOML：

- `devkit init -p` 打印生成的策略。
- `devkit tui` 打开 TUI 编辑器，并把接受后的策略写入 `devkit.toml`。
- `devkit tui -p` 打开 TUI 编辑器，然后打印接受后的策略。
- `devkit init -i -p` 打开 TUI 编辑器，然后打印接受后的策略。
- `devkit init --output ./devkit.toml` 写入 starter 文件。
- `devkit init --force` 覆盖已有文件。
- 生成的策略包含 stable channel、当前平台、检测到的核心运行时，以及少量已安装 CLI 包。

TUI 中，左侧面板选择区域，中间面板编辑字段，右侧面板查看生成的 TOML。主要按键包括：`Up`/`Down` 或 `j`/`k` 移动，`Left`/`Right` 或 `Tab` 切换面板，`Space` 启用或禁用工具，`Enter` 或 `e` 编辑字段，`P` 打开完整预览，`PageUp`/`PageDown` 滚动预览，`S` 保存，`Q` 取消。`Esc` 不作为破坏性的关闭快捷键，以避免某些终端把方向键前缀作为 `Esc` 发送时误退出。编辑 Node、Go 或 Rust 版本字段时，会在可用时打开基于远程 release 列表的版本选择器；可以选择 `24.x` 这样的主版本选择器、精确版本，或按 `C` 输入自定义值，例如 `24`；按 `Q` 关闭选择器。TUI 渲染在 stderr，因此 `devkit tui -p > devkit.toml` 在保存后仍然只会把 TOML 写入文件。

## 最新版本来源

`upgrade --dry-run` 可以查询已知上游来源：

- npm registry：`npm`、`pnpm`、`yarn`、`wrangler`
- 当 Homebrew 是配置的管理器时，使用 Homebrew formula metadata：`fnm`、`bun`、`deno`、`python`、`poetry`、`ruby`、`brew`
- 当 `node` 由 `fnm` 或 `nvm` 管理时，使用 Node remote releases
- 当 `go` 使用官方来源时，使用 Go official endpoint
- GitHub releases page：`uv`
- rustup：`rustup`、`rustc`、`cargo`
- 不支持的 manager-specific 查询，例如 `winget`、`scoop`、`choco` 和发行版包管理器，会报告为不可用，而不是回退到错误的数据来源

MVP 会打印命令而不是直接应用升级。这样在策略模型继续演进时，对个人和团队机器更安全。

## 配置

参考 `examples/devkit.toml`。它展示了一个单文件团队策略，包含共享默认值以及 `[platform.macos]`、`[platform.linux]`、`[platform.windows]` 覆盖。`DevkitConfig::read` 会自动为当前机器解析有效策略；先应用 OS 级覆盖，再用 `[platform.macos-arm64]` 这样的精确平台标签进一步细化。

`devkit config validate` 会检查当前有效策略和显式平台覆盖段，覆盖内容包括不支持的工具、无效的 manager/source 选择、未知平台 key、空 package 条目，以及运行时包管理器缺口。`devkit config explain` 会打印每个字段解析后的来源，例如 `base`、`platform.macos` 或 `platform.macos-arm64`。
