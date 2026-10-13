# 安装

## 环境要求

| 依赖 | 说明 |
| --- | --- |
| Rust 1.96.1 或更新 | 构建 `agt` CLI，整个框架都在其中 |
| Node.js 22 或更新 | 运行 Web 控制台；不需要安装 npm 依赖 |
| 模型接口 | 任意 OpenAI 兼容 API，模型需支持工具调用 |
| Codex CLI（可选） | 仅 [Codex Agent](./codex.md) 需要；agt 以 `codex app-server` 方式启动它 |

agt 可在 Linux 和 macOS 上运行。在 Windows 上需要显式告诉 Web 控制台 CLI 的路径（见下文）。

## 从源码构建

```bash
git clone https://github.com/blacsheep/agt.git
cd agt
cargo build -p agt-cli
```

生成的程序是 `target/debug/agt`。也可以把 `agt` 安装到 `PATH`：

```bash
cargo install --path crates/agt-cli
agt --version
```

## 启动 Web 控制台

在仓库根目录执行：

```bash
node web/server.mjs
```

如果用 `cargo install` 安装了 `agt`，让控制台使用它：

```bash
node web/server.mjs --binary "$(command -v agt)"
```

Windows PowerShell：

```powershell
node web/server.mjs --binary .\target\debug\agt.exe
```

服务会打印一个链接，例如 `http://127.0.0.1:4317/#<令牌>`。请打开**完整**链接，包括 `#` 后面的令牌，
浏览器靠它获得授权。服务同时会在后台启动调度器，配置好的 Agent 可以立即回复。

| 参数 | 默认值 | 用途 |
| --- | --- | --- |
| `--port` | `4317` | 监听端口（始终只监听 `127.0.0.1`） |
| `--home` | `$AGT_HOME`，否则 `~/.agt` | 使用的数据目录 |
| `--binary` | `target/debug/agt` | 调用的 `agt` CLI |
| `--principal` | `web-operator` | 你在会话中的身份 |

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
  runtime/runtime.db   # 绑定、执行、任务、审批、Codex 会话
```

如果想把数据放在别处，在 shell 配置文件中把 `AGT_HOME` 设为绝对路径：

```bash
export AGT_HOME=/data/agt
```

home 的选择顺序是：`--home` > `AGT_HOME` > `~/.agt`。`--home` 只对传入它的那条命令生效，长期切换请用
`AGT_HOME`。要为某个项目单独建立 home，运行 `agt init <目录>`。

## 更新

```bash
git pull
cargo build -p agt-cli
```

然后重启 `node web/server.mjs`，用新链接重新打开页面。Web 控制台、CLI 和调度器必须使用同一次构建。

## 下一步

继续阅读[快速开始](./quick-start.md)。
