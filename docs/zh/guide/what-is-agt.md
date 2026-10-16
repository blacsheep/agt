# agt 是什么？

agt 在你自己的机器上运行一组 AI Agent。它背后的想法是：管理资源（Prompt、工具、Agent）本身也是一种能力，可以交给 Agent。拥有这项能力的 Agent 能写 Prompt、注册工具、创建其他 Agent 并拉进群聊，然后和这支团队一起干活。

通常你只和一个 Agent 对话，也就是 *lead*。你说明目标，lead 判断需要哪些角色，创建每个成员，缺工具时找编程 Agent 来做，并协调整个过程。你可以旁观、插话，也可以审批它的操作。

大多数 Agent 框架要求你在运行前用代码定义好 Agent、工具和它们之间的流程。agt 把这件事交给 lead，它创建的一切都以文件形式保存，你可以查看和修改。

## 能用它做什么

| 功能 | 带来什么 | 指南 |
| --- | --- | --- |
| 多种模型接口 | 接入 OpenAI 或兼容的网关、本地服务，以及 Anthropic、Google Gemini | [接入模型](./models.md) |
| Agent 即配置 | Agent 由模型、Prompt、Skill、MCP 服务和内置工具组合而成 | [创建 Agent](./agents.md) |
| 私聊与群聊 | 在 IM 中和 Agent 对话；Agent 回复私聊和 @提及 | [对话与自动回复](./conversations.md) |
| 会自己组队的 lead | 内置 Prompt，知道如何创建和协调 Agent | [让 lead 组建团队](./lead-team.md) |
| MCP 工具 | 给 Agent 挂载 stdio 或 HTTP MCP 服务，或让 Agent 自己注册新服务 | [MCP 工具](./mcp.md) |
| 编程 Agent | 让 Agent 运行在能改代码的 Codex、[Claude Code](./claude-code.md)、[DeepSeek Harness](./deepseek-harness.md) 或其他 ACP Agent 上 | [Codex Agent](./codex.md) |
| 可追踪任务 | 带排队、等待、结果和汇报的工作委派 | [任务协作](./tasks.md) |
| 记忆 | Agent 搜索自己的历史，并保存关联原始消息的笔记 | [记忆](./memory.md) |
| 本机命令 | 可选地允许 Agent 在宿主机上执行 Shell 命令 | [本机命令](./local-execution.md) |
| 审批 | 人工、审核模型或全部放行，逐个审核工具调用 | [权限审批](./permissions.md) |
| 外部客户端接入 | 把 agt 的工具开放给任意 MCP 客户端，例如编辑器或其他 Agent | [在 MCP 客户端中使用 agt](./mcp-clients.md) |

以上功能在 Web 控制台和 `agt` CLI 中都能使用。控制台也有[桌面应用](./desktop.md)版本。

## 各部分如何配合

![你在会话中对话，调度器执行 Agent，Agent 回复、修改资源并调用工具](../../public/guide/how-it-fits.zh.svg)

- **资源**是描述 Agent 的文件：Provider、Model、Prompt、Skill、MCP 服务、Harness 和 Agent 定义。
- **会话**是人和 Agent 之间的私聊与群聊。
- **调度器**决定哪些 Agent 应该回复一条消息、执行它们，并在重启后保留待完成的工作。
- 拥有 `resource_management` 能力的 Agent 可以修改资源，lead 正是这样组建自己的团队的。

[核心概念](./concepts.md)会更详细地解释这些内容。

## 设计原则

- Agent 只能做挂在它身上的工具允许的事。Prompt 里的指令算不上权限控制，agt 也不靠它来限制 Agent。新建的 Agent 不会继承创建者的工具。
- 会话、执行、任务和审批都保存在 SQLite 中。重启后未完成的工作会继续；无法确认结果的工作会标记出来交给你，不会被悄悄重跑。
- 所有逻辑都在 Rust 核心里。Web 控制台是 `agt` CLI 之上的本地界面，一边能做的事，另一边也能做。

## agt 不是什么

agt 是供单个操作者使用的可信本地工具。Web 控制台只监听 `127.0.0.1`，持有链接的人可以管理所有资源。它不是托管服务，也不支持多租户。见[安全模型](../help/security.md)。

## 下一步

- [安装 agt](./installation.md)
- [快速开始](./quick-start.md)：从全新安装到一个会自己组队的 lead
