# 安装

## 环境要求

| 依赖 | 说明 |
| --- | --- |
| Linux、macOS 或 Windows | 提供 x86_64 和 ARM64 的预编译程序（Windows 为 x86_64） |
| 模型接口 | OpenAI 或兼容服务、Anthropic、Google Gemini，模型需支持工具调用；见[接入模型](./models.md) |
| 编程 Agent 工具（可选） | 仅运行在这些工具上的 Agent 需要：[Codex Agent](./codex.md) 需要 Codex CLI，以及 [Claude Code](./claude-code.md)、[DeepSeek Harness](./deepseek-harness.md) |

## 安装

agt 有两种形式，两者使用同一个[工作空间](#数据存放位置)，可以只用一种，也可以都用：

- **命令行（`agt`）**：用下面一条命令安装，已经内置 Web 控制台（`agt web`）。
- **桌面应用（AGT Studio）**：支持 macOS、Windows 和 Linux，从[最新发布](https://github.com/blacsheep/agt/releases/latest)下载对应系统的安装包。见[桌面客户端](./desktop.md)。

本页下面的内容都是关于命令行的。

**macOS 和 Linux：**

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/blacsheep/agt/releases/latest/download/agt-cli-installer.sh | sh
```

**Windows（PowerShell）：**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/blacsheep/agt/releases/latest/download/agt-cli-installer.ps1 | iex"
```

安装脚本会识别你的平台，从 [GitHub Releases](https://github.com/blacsheep/agt/releases) 下载对应的 `agt` 程序，校验 SHA-256，然后放到 `~/.local/bin`（Windows 为 `%USERPROFILE%\.local\bin`）。它还会把该目录加入 `PATH`；打开一个新终端，或运行它打印的 `source` 命令，然后检查：

```bash
agt --version
```

::: details 想先看看脚本内容？
先下载、阅读，再运行：

```bash
curl --proto '=https' --tlsv1.2 -LsSf -o agt-installer.sh https://github.com/blacsheep/agt/releases/latest/download/agt-cli-installer.sh
less agt-installer.sh
sh agt-installer.sh
```

也可以从 [Releases 页面](https://github.com/blacsheep/agt/releases)下载对应平台的压缩包，用 `sha256.sum` 校验后，把 `agt` 放到 `PATH` 中的任意位置。
:::

### 其他安装方式

**用 Cargo**（Rust 1.96.1 或更新），从最新源码安装：

```bash
cargo install --git https://github.com/blacsheep/agt agt-cli --locked
```

**从克隆的仓库构建**，用于开发：

```bash
git clone https://github.com/blacsheep/agt.git
cd agt
cargo build -p agt-cli        # target/debug/agt
```

## 启动 Web 控制台

```bash
agt web
```

命令会打印一个链接，例如 `http://127.0.0.1:4317/#<令牌>`。请打开**完整**链接，包括 `#` 后面的令牌，浏览器靠它获得授权。控制台同时运行调度器，配置好的 Agent 可以立即回复。按 Ctrl+C 停止。

| 选项 | 默认值 | 用途 |
| --- | --- | --- |
| `--port` | `4317` | 监听端口（始终只监听 `127.0.0.1`） |
| `--principal` | `web-operator` | 你在会话中的身份 |
| `--home`（写在 `web` 前面） | `$AGT_HOME`，否则 `~/.agt` | 使用的数据目录 |

控制台已经内置在 `agt` 中，不需要其他依赖。在克隆的仓库中，可以用 `cargo run -p agt-cli -- web` 从源码启动。

### 在远程服务器上运行

控制台只监听服务器自己的 `127.0.0.1`。在你的电脑上建立 SSH 隧道：

```bash
ssh -N -L 4317:127.0.0.1:4317 user@server
```

然后在本地浏览器打开服务器上打印的完整链接。不要把端口暴露到公网：令牌等同于整个工作空间的控制权。

## 数据存放位置

agt 把所有数据放在一个 *home* 目录中，默认是 `~/.agt`，首次使用时自动创建：

```text
~/.agt/
  config.yaml          # 工作空间设置（调度、权限、记忆、本机命令）
  providers/ models/ prompts/ skills/ mcps/ agents/ harnesses/   # 资源
  im/im.db             # 会话和消息
  runtime/runtime.db   # 绑定、执行、任务、审批、Harness 会话
```

如果想把数据放在别处，在 shell 配置文件中把 `AGT_HOME` 设为绝对路径：

```bash
export AGT_HOME=/data/agt
```

home 的选择顺序是：`--home` > `AGT_HOME` > `~/.agt`。`--home` 只对传入它的那条命令生效，长期切换请用 `AGT_HOME`。要为某个项目单独建立 home，运行 `agt init <目录>`。

## 更新

有新版本时 agt 会告诉你，但在你要求之前不会做任何改动：

- 在终端里运行命令时，最多每天一次，结尾会多一行提示，例如 `agt 0.6.2 已发布（当前 0.6.1），运行 agt upgrade 升级`。检查和命令同时进行，结果缓存一天。输出 JSON、输出不是终端（脚本和 agent 调用）以及在 CI 中时不检查。
- Web 控制台的导航底部会出现**新版本**按钮，点开后给出要运行的命令。

升级：

```bash
agt upgrade --check   # 只查看有没有新版本
agt upgrade           # 显示两个版本号，确认后再替换 agt
```

`agt upgrade` 会把新版本的安装脚本运行到上次安装的目录，下载和校验方式与首次安装相同。加 `--yes` 可跳过确认。升级后重启 `agt web` 才会用上新版本。home 目录中的数据会保留。

用 Cargo 安装或从克隆构建的 `agt` 没有安装记录，`agt upgrade` 不会改动它，并提示你按原来的方式升级。任何安装方式都可以再运行一次安装脚本来升级。

设置 `AGT_NO_UPDATE_CHECK=1` 可以关掉提示和控制台的按钮。

## 卸载

删除程序（`~/.local/bin/agt`）以及安装脚本创建的 `PATH` 配置（`~/.config/agt-cli/`，以及 shell 配置文件中引用它的那一行）。数据会保留在 `~/.agt`（或 `$AGT_HOME`）中，直到你自己删除。

## 下一步

继续阅读[快速开始](./quick-start.md)。
