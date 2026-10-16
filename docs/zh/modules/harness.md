# Harness

| 概览 | |
| --- | --- |
| **是什么** | 外部执行引擎（Codex、Claude Code 或 DeepSeek Harness）及其工作的项目目录 |
| **CLI** | `agt harness` |
| **Agent 工具** | `kind: "harness"` 的 `resource_*`、`harness_models`、`harness_session_*` |
| **Web 控制台** | **资源库** → Harness；Agent 表单中的**外部 Harness**；会话中的 `/model`、`/status` 等命令 |
| **文件** | `harnesses/<category>/<name>/harness.yaml` |
| **Crate** | `agt-harness` |

## 作用

Harness 让 Agent 运行在 [Codex](https://github.com/openai/codex) 上，而不是 agt 自己的模型循环。Codex 自带文件编辑、Shell 命令和沙箱，因此 Codex Agent 可以编写和测试代码，包括供团队其他成员使用的 MCP 服务。

agt 启动 `codex app-server`，为每个 Agent 在每个会话中保持一个 Codex 会话，把 Codex 的审批请求转发到 [审批队列](./permissions.md)，并把 Codex 的最终答复发布为 Agent 的回复。Codex 使用自己的登录和配置；agt 不保存 Codex 凭据。

Harness 也可以改用 [Claude Code](../guide/claude-code.md)（`"engine": "claude-code"`）。agt 以无界面模式运行它，每一轮一个进程，以进程内 MCP 服务的方式把 agt 工具提供给它，并把它的权限请求发到同一个审批队列。[DeepSeek Harness](../guide/deepseek-harness.md)（`"engine": "deepseek-harness"`）通过 Agent Client Protocol 驱动，每一轮把 agt 工具作为本机 HTTP MCP 服务挂载给它。

## 怎么用

1. 安装 Codex，并以运行 agt 的用户执行 `codex login`。
2. 创建 Harness：

```bash
agt harness resource create toolsmith --file ./toolsmith.json
agt harness resource test toolsmith
agt harness models toolsmith
```

3. 基于它创建 Agent：`agt agent create toolsmith --harness toolsmith --prompt toolsmith`。
4. 在私聊中和它对话，或在群里提及它。

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `working_directory` | Codex 工作的项目 |
| `sandbox` | `read_only` 或 `workspace_write` |
| `model`、`reasoning_effort` | 新会话的默认值 |

## 全部参数

### 文件：`harness.yaml`（通过 CLI 创建时使用 JSON）

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

| 字段 | 必填 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `engine` | 是 | | `codex`、`claude-code` 或 `deepseek-harness` |
| `executable` | 是 | | 已存在的 Codex、Claude Code 或 `dsh` 程序的绝对路径 |
| `working_directory` | 是 | | 已存在目录的绝对路径 |
| `sandbox` | 是 | | `read_only` 或 `workspace_write` |
| `model` | 否 | Codex 的默认值 | 新会话的默认模型 |
| `reasoning_effort` | 否 | Codex 的默认值 | 新会话的默认推理档位 |
| `timeout_seconds` | 否 | `1800` | 单轮时间上限，10 到 86400 |
| `description` | 否 | | 最多 4096 字节 |

未知字段会被拒绝。

### `agt harness resource`

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `create <名称>` | `--file <json>`（完整对象）、`--category <c>` | 创建 Harness |
| `update <名称>` | `--file <json>`（要修改的字段）、`--category <c>` | 更新 Harness |
| `test <引用>` | `--category <c>` | 检查配置，不运行模型 |
| `list`、`info`、`delete`、`category` | | 见[资源通用操作](./resources.md#通用-cli-命令) |

### 查看 Codex

| 命令 | 含义 |
| --- | --- |
| `agt harness models <引用>` | Codex 为该 Harness 报告的模型和推理档位 |
| `agt harness skills <引用>` | Codex 在项目中找到的 Skill |
| `agt harness mcp <引用>` | Codex 中配置的 MCP 服务及其状态 |
| `agt harness capabilities` | agt 支持哪些引擎操作 |

### `agt harness session`

每个 Agent 在每个会话中有一个 Codex 会话。会话保留其开始时的配置；修改 Harness 只影响新会话。

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `list` | | 所有会话 |
| `current` | `--principal <agent>` `--conversation <id>` | 获取或准备某个 Agent 在某个会话中的 Codex 会话（不启动 Codex） |
| `info <id>` | | 会话详情 |
| `models <id>` | | 该会话可用的模型 |
| `skills <id>`、`mcp <id>` | | 该会话看到的 Skill 和 MCP 服务 |
| `configure <id>` | `--model <模型>`（必填）、`--effort <档位>` | 修改该会话的模型和推理档位，从下一轮生效 |
| `rename <id> <标题>` | | 为 Codex 线程命名 |
| `compact <id>` | | 让 Codex 压缩上下文并等待完成（可能调用模型） |
| `cancel <id>` | | 停止正在进行的一轮。已做的文件修改会保留。 |

### 会话命令

`/model`、`/status`、`/permissions`、`/compact`、`/rename`、`/skills`、`/mcp`、`/new`、`/stop`。见 [Web 控制台](./web-console.md#输入框命令)。

### Agent 工具

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `harness_models` | `reference` | Harness 的模型和推理档位 |
| `harness_session_get` | `conversation`, `target` | Agent `target` 在某个会话中的 Codex 会话 |
| `harness_session_models` | `conversation`, `target`, `session` | 该会话的可选项 |
| `harness_session_configure` | `conversation`, `target`, `session`, `model`, `effort?` | 修改该会话的模型和推理档位 |

这些工具需要 `resource_management`。调用者和 `target` 都必须是该会话的成员。正在执行或需要处理的会话会拒绝修改。

### Codex Agent 支持的功能

| 支持 | 不支持 |
| --- | --- |
| 角色 Prompt 和 Skill | agt 的 MCP 资源 |
| `resource_management`、`task_collaboration`、消息、记忆、任务 | `local_execution` |
| 响应策略 `mentioned` 和 `manual` | 响应策略 `all` |
| 流式预览、`/stop`、`/new` | `/review`、`/diff`、导入或分叉 Codex 线程 |

## 相关模块

- [Agent](./agent.md)：选择 Harness 而不是 Model
- [权限审批](./permissions.md#全部参数)：审批 Codex 的请求
