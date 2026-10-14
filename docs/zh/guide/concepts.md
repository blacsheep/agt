# 核心概念

本页解释文档其余部分用到的几个基本概念。

## Home

所有数据都在一个目录中，即 *home*（默认 `~/.agt`，或 `$AGT_HOME`）。其中有资源文件、会话数据库和运行时数据库。不同的 home 是完全独立的工作空间。见[安装](./installation.md#数据存放位置)。

## 资源

资源是组成 Agent 的基本单元，每个资源都是 home 中的一个小文件：

| 资源 | 文件 | 描述的内容 |
| --- | --- | --- |
| Provider | `provider.yaml` | 一个 OpenAI 兼容接口及其 API Key |
| Model | `model.yaml` | Provider 上的一个模型：协议、温度、超时、上下文窗口 |
| Prompt | `PROMPT.md` | 角色描述 |
| Skill | `SKILL.md` | 可复用的指令包，会加入系统提示词 |
| MCP | `mcp.json` | 如何启动或连接一个提供工具的 MCP 服务 |
| Harness | `harness.yaml` | 外部执行引擎（Codex）及其工作的项目目录 |
| Agent | `agent.yaml` | 一个 Model 或 Harness，加上 Prompt、Skill、MCP 服务和内置工具 |

资源用 `category/name` 引用，例如 `default/lead`。只写 `name` 表示 `default` 分类。分类用来给资源分组，比如每个项目一个分类。

修改资源会在下次加载时生效：降低某个 Model 的温度后，所有使用它的 Agent 在下一次执行时都会用新值。已经开始的执行保留启动时的配置。

## Agent、身份与绑定

**Agent 定义**只是配置。要参与会话，它还需要：

- 一个**身份**（*principal*）：聊天系统中的参与者，例如 `lead`。人也是身份；Web 控制台的操作者默认是 `web-operator`。
- 一个**绑定**：把身份和定义连接起来，并保存运行设置：启用或停用、工作目录和响应策略。

![Agent 定义通过绑定连接到身份，身份是会话的成员](../../public/guide/agent-binding.zh.svg)

Web 控制台的**创建 Agent**会一次创建这三样。在 CLI 和 Agent 工具中，它们是分开的步骤： `resource_write`（定义）、`principal_create`（身份）和 `agent_bind`（绑定）。停用绑定会让 Agent 在所有会话中停止工作，但保留它的身份和历史。

## 执行引擎

每个 Agent 定义选择一种引擎：

- **Native**（`model`）：agt 自己的模型与工具循环。支持 MCP 服务、Skill、全部内置工具和审批策略。
- **Codex**（`harness`）：把 Agent 作为项目目录中的 [Codex](https://github.com/openai/codex) 会话运行。Codex 自带文件编辑、Shell 和沙箱。

## 内置能力

Agent 只拥有你给它的能力。除了挂载的 MCP 服务，还有三组内置能力，默认全部关闭：

| 能力 | 允许做什么 |
| --- | --- |
| `resource_management` | 读取、创建、修改和删除资源；创建 Agent 身份；绑定、启用和停用 Agent |
| `task_collaboration` | 向其他 Agent 委派可追踪任务、等待结果、提交汇报 |
| `local_execution` | 在宿主机上执行 Shell 命令（还需要工作空间设置） |

每个托管 Agent 还固定拥有消息、会话和[记忆](./memory.md)工具。能力不会继承：创建其他 Agent 的 Agent 决定新 Agent 获得什么。见 [Agent 模块](../modules/agent.md#内置能力)。

## 会话

**会话**是两个参与者之间的私聊，或一个群聊。每条消息都会投递给其他所有成员；Agent 是否回复取决于它的**响应策略**：

| 策略 | 私聊 | 群聊 |
| --- | --- | --- |
| `mentioned`（默认） | 回复 | 被 @提及时回复 |
| `all` | 回复 | 考虑每条消息 |
| `manual` | 不自动回复 | 不自动回复 |

没有话要说的 Agent 会只回复 `[[silent]]`，此时不会发布任何内容。

## 讨论与调度器

Agent 不会自己运行。人在开启自动回复的情况下发送消息（或执行 `agt schedule start`）时，就开始一轮**讨论**。讨论授权 Agent 在其中消耗模型调用，但有限制：

- 每个 Agent 在一轮讨论中最多回复 100 次（你发送新消息会重置计数）；
- 授权持续 30 分钟，之后不再开始新的工作；
- 同时最多运行 3 个 Agent。

这三项限制都可以调整，见[调度器](../modules/scheduler.md#设置-config-yaml)。你可以随时暂停、继续或结束讨论。

**调度器**是把消息转成执行的后台进程。它由 Web 控制台启动，或通过 `agt schedule run --watch` 启动。它的状态保存在磁盘上，所以等待中的工作会在重启后继续。

## 执行（Run）

一次**执行**是 Agent 的一次运行：为回答一条消息或完成一项任务而进行的模型调用、工具调用和最终回复。执行记录保存在 `runtime/runtime.db` 中。可以在 Web 控制台（调试模式）或用 `agt run list`、`agt run info <id>` 查看。

执行状态 `completed` 表示模型正常结束，不代表目标已经达成。如果执行在工具调用中途被中断，agt 会把它标为中断，而不是重新调用工具，因为工具可能已经产生了副作用。

## 任务

普通消息是对话。当一个 Agent 需要一个可靠的结果时，它会发出**任务**：一个可追踪的请求，有状态（`queued`、`running`、`succeeded`、`failed` 等）、会送回请求者的结果，以及发回原会话的明确汇报。见[任务协作](./tasks.md)。

## 审批

Native Agent 执行工具之前，调用要经过工作空间的**审批策略**：模型审核、人工审批或全部放行。Codex Agent 会发出自己的审批请求，单独处理。见[权限审批](./permissions.md)。
