# Codex Agent

Codex Agent 运行在 [Codex](https://github.com/openai/codex) 会话上，而不是 agt 自己的模型循环中。
Codex 自带文件编辑、Shell 命令和沙箱，因此 Codex Agent 适合编写和测试代码，包括其他 Agent 要用的 MCP 服务。

## 环境要求

- 运行 agt 的机器上已安装 Codex CLI。agt 以 `codex app-server` 方式启动它。
- 在运行 agt 的同一用户账号下登录 Codex（`codex login`）。agt 不保存 Codex 凭据，而是沿用 Codex 自己的登录和配置，
  包括其中配置的自定义模型 Provider。

## 创建 Harness

**Harness** 资源描述如何启动 Codex、在哪里工作：

```json
{
  "engine": "codex",
  "executable": "/usr/local/bin/codex",
  "working_directory": "/home/me/projects/tools",
  "sandbox": "workspace_write",
  "model": "gpt-5.5",
  "reasoning_effort": "medium",
  "timeout_seconds": 1800,
  "description": "为团队构建 MCP 服务"
}
```

| 字段 | 必填 | 含义 |
| --- | --- | --- |
| `engine` | 是 | 目前只支持 `codex` |
| `executable` | 是 | Codex 程序的绝对路径 |
| `working_directory` | 是 | Codex 工作的项目目录的绝对路径 |
| `sandbox` | 是 | `read_only` 或 `workspace_write` |
| `model`、`reasoning_effort` | 否 | 新会话的默认值；省略则使用 Codex 的默认值 |
| `timeout_seconds` | 否 | 单轮的时间上限，10–86400（默认 1800） |
| `description` | 否 | 在控制台中显示的说明 |

**Web 控制台：** **资源库** → **新建资源** → Harness。保存后，可用**读取可用模型**查看 Codex 报告的模型和推理档位，
用**检查连接**测试配置，不会调用模型。

**CLI：**

```bash
agt harness resource create toolsmith --file ./toolsmith.json
agt harness resource test toolsmith
agt harness models toolsmith           # Codex 报告的模型和推理档位
```

## 创建 Agent

选择 Harness 而不是 Model：

```bash
agt agent create toolsmith --harness toolsmith --prompt toolsmith
```

在 Web 表单中，把**执行方式**设为**外部 Harness** 并选择 Harness。之后像其他 Agent 一样和它对话：私聊，或在群里提及它。

## Codex Agent 能用和不能用的功能

| 可用 | 不可用 |
| --- | --- |
| 角色 Prompt 和 Skill（作为指令发给 Codex） | agt 的 MCP 资源（请改在 Codex 中配置 MCP） |
| `resource_management` 和 `task_collaboration` 工具 | `local_execution`（Codex 有自己的 Shell） |
| 消息、记忆和任务工具 | 响应策略 `all`（只支持 `mentioned` 和 `manual`） |
| 流式预览、`/stop` 和 `/new` | |

Codex 需要超出沙箱时会请求审批。这些请求和 Native 工具调用进入同一个审批队列，按 **Harness** 审批模式处理；
见[权限审批](./permissions.md#codex-审批)。

## 会话

每个 Codex Agent 在每个会话中保持一个 Codex 会话。这个会话会记住它开始时使用的模型和推理档位：

- 修改 Harness 默认值只影响新会话。
- 在会话中使用 `/model` 只修改该 Agent 在这个会话中的模型和推理档位。
- `/new` 结束当前会话，用当前默认值开始新会话。聊天记录和项目目录中的文件会保留。

Codex Agent 的其他命令：`/status`、`/permissions`、`/compact`、`/rename`、`/skills`、`/mcp`。
见[对话与自动回复](./conversations.md#输入框命令)。

在 CLI 中，`agt harness session list` 列出会话，`agt harness session cancel <id>` 停止正在运行的一轮。

## 注意事项

- **文件是共享的。** 使用同一工作目录的 Agent 编辑的是同一批文件。多个 Codex Agent 并行工作时，给每个 Agent 单独的目录或 git worktree。
- **停止不等于撤销。** 取消一轮会停止 Codex，但已做的文件修改会保留。如果一轮的结果不明确，状态会显示**需要处理**：
  请检查项目文件，然后使用 `/new`。
- **尚不支持：** `/review`、`/diff`、导入或分叉已有的 Codex 线程。
