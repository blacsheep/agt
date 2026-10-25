# Claude Code Agent

Claude Code Agent 运行在 [Claude Code](https://claude.com/claude-code) 上，而不是 agt 自己的模型循环。和 [Codex Agent](./codex.md) 一样，它可以在项目目录里改文件、执行命令，也能使用 agt 的消息、记忆和任务工具。

## 前提

- 运行 agt 的机器上已经安装 Claude Code（`command -v claude`）。
- 用运行 agt 的同一个系统账号登录 Claude Code。agt 不保存 Claude 的凭据，沿用 Claude Code 自己的登录和设置。

agt 以无界面模式运行 Claude Code（`claude -p`，stream-json 协议），每一轮启动一个进程，下一轮接着用这个 Agent 的会话。

## 创建 Harness

```json
{
  "engine": "claude-code",
  "executable": "/home/me/.local/bin/claude",
  "working_directory": "/home/me/projects/tools",
  "sandbox": "workspace_write",
  "model": "opus",
  "reasoning_effort": "high",
  "timeout_seconds": 1800,
  "description": "编写并审查代码"
}
```

| 字段 | 对 Claude Code 的含义 |
| --- | --- |
| `sandbox` | `workspace_write`：项目内的文件修改直接接受（`--permission-mode acceptEdits`），执行命令等其他操作需要审批。`read_only`：禁用编辑类工具，所有操作都需要审批。 |
| `model` | Claude Code 的模型别名（`fable`、`opus`、`sonnet`、`haiku`）或完整模型名。留空使用 Claude Code 的默认模型。 |
| `reasoning_effort` | `low`、`medium`、`high`、`xhigh` 或 `max`，作为 `--effort` 传入。 |

**Web 控制台：** **资源库** → **新建资源** → Harness，执行引擎选择 **Claude Code**。

**CLI：**

```bash
agt harness resource create reviewer --file ./reviewer.json
agt harness resource test reviewer      # 检查可执行文件确实是 Claude Code，不调用模型
```

然后和 Codex 一样在它上面创建 Agent：`agt agent create reviewer --harness reviewer --prompt reviewer`。

## agt 怎么和 Claude Code 对接

- **指令：** Agent 的角色 Prompt 和 Skills 追加到 Claude Code 的系统提示词里（`--append-system-prompt`）。
- **agt 工具：** agt 的消息、记忆、任务和资源工具，以 agt 进程内一个名为 `agt` 的 MCP 服务提供给 Claude Code，显示为 `mcp__agt__…`。Claude Code 调用它们时不会再请求审批，由 agt 自己检查权限。
- **审批：** Claude Code 想做权限模式不允许的事时，请求会进入 agt 的审批队列，按 **Harness** 审批模式处理，见[权限审批](./permissions.md#harness-审批)。可以批准本次、拒绝或取消这一轮。
- **会话：** 每个 Agent 在每个会话里保留一个 Claude Code 会话，每一轮接着用。`/new` 开始新会话。

## Claude Code Agent 能用和不能用的

| 可用 | 不可用 |
| --- | --- |
| 角色 Prompt 和 Skills | agt 的 MCP 资源（请在 Claude Code 里配置 MCP） |
| `resource_management` 和 `task_collaboration` 工具 | `local_execution`（Claude Code 有自己的终端） |
| 消息、记忆和任务工具 | 响应策略 `all`（只支持 `mentioned` 和 `manual`） |
| `/stop`、`/new`、`/status`，回复实时预览 | `/compact`、`/rename`、`/skills`、`/mcp`、`/permissions` |

## 注意事项

- **文件是共享的。** 使用同一工作目录的 Agent 会改同一批文件。
- **停止不等于撤销。** 取消一轮会停下 Claude Code，但已经做出的修改会保留。
- **你的 Claude Code 设置会生效。** 工作目录里的项目设置、用户设置、CLAUDE.md 和 hooks 都会像在终端里一样加载。
