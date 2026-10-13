# 快速开始

本指南带你从全新安装走到一个会自己组建团队的 lead Agent。主要使用 Web 控制台；
最后的 [CLI 部分](#用-cli-完成同样的事)展示在终端中的同样步骤。

开始之前，先[安装 agt](./installation.md)，用 `node web/server.mjs` 启动控制台，并打开它打印的完整链接。

## 1. 接入模型

在左侧栏打开**资源库**，点击**新建资源**。

1. 创建一个 **Provider**：名称、接口的 base URL（例如 `https://api.openai.com/v1`）和 API Key。
2. 在该 Provider 上创建一个 **Model**。点击**获取模型列表**从 Provider 提供的模型中选择，或直接填写模型名。
   选择 Provider 对该模型支持的 API：**Chat Completions** 或 **Responses**。保持工具调用开启。

所有选项见[接入模型](./models.md)。

## 2. 选择工具调用的审批方式

默认情况下，Agent 的每一次工具调用都要等待审批。打开**运行设置** → **工具权限审批**，设置
**Native 权限审核策略**：

| 模式 | 适用场景 |
| --- | --- |
| **模型审核，异常转人工** | 由审核模型批准常规调用，有疑问的转给你。在**审核模型**中选择第 1 步创建的模型。 |
| **人工审批** | 每次调用都由你批准。最安全，但 lead 的调用很多。 |
| **自动放行** | 不审批。只适合在可随时丢弃的 home 中做实验。 |

没有配置审核模型时，模型审核会转为人工审批。待审批的请求显示在会话输入框上方，可以在那里批准或拒绝。
见[权限审批](./permissions.md)。

## 3. 创建 lead

打开 **Agent 团队**，点击**创建角色**：

- **资源名称 / 身份 ID**：`lead`
- **执行方式**：**AGT 内置**，选择第 1 步的模型
- **角色 Prompt**：**使用已有 Prompt**，选择 `default/lead`
- **框架内置工具**：勾选**资源管理**和**任务协作**
- **响应策略**：保持**私聊 / 群内被提及**

保存后会创建 Agent 定义、聊天身份和启用它的绑定，lead 就可以对话了。

## 4. 给它一个目标

点击**新建会话**，选择和 `lead` 私聊，描述你想要的结果，例如：

> 为这个仓库组建一个两人代码审查小组：一人关注正确性，一人关注可读性。把他们和我拉进一个群，让他们审查最近的改动。

保持**发送后自动回复**开启。发送即授权 Agent 运行，会调用你的模型并产生费用。

lead 会先查看已有资源，然后编写角色 Prompt、创建并启用每个队友、建群并向团队布置任务。在它工作时：

- 输入框上方的执行横条显示每个正在运行的 Agent 及其工具调用。
- 如果选择了人工审批，待审批请求也会出现在那里。
- **Agent 团队**会随着创建列出新的队友。点击某个队友可以查看它的配置、状态和最近执行。
- 打开右上角的**调试模式**，可以看到执行 ID 和错误码。

## 5. 让它自己造工具

如果有可用的 [Codex Agent](./codex.md)，lead 还能自己造工具。让它做一件需要新工具的事，
例如为桌游或某个数据源写一个小型 MCP 服务。lead 会把代码交给 Codex Agent 编写，把结果注册为 MCP 服务，
测试它能正常启动，再挂载给需要它的 Agent。见[让 lead 组建团队](./lead-team.md)。

## 用 CLI 完成同样的事

控制台的所有操作都经过 `agt` CLI。下面的例子手动创建一个审查 Agent 并和它对话；lead 通过工具完成的也是这些步骤。

```bash
export MODEL_API_KEY=sk-...
agt provider create main --base-url https://api.openai.com/v1 --api-key-env MODEL_API_KEY
agt model create fast --provider main --model gpt-5-mini --api-format responses --tool-call true
agt prompt create reviewer --text '你负责审查代码改动。没有需要补充的内容时，只回复 [[silent]]。'
agt agent create reviewer --model fast --prompt reviewer

# 聊天身份：你自己和 Agent，然后启用 Agent
agt im principal create me --name Me
agt tool call --principal me --manager principal_create --args '{"id":"reviewer","name":"Reviewer","kind":"agent"}'
agt tool call --principal me --manager agent_bind --args '{"principal":"reviewer","agent":"reviewer"}'

# 打开私聊并发送消息
agt tool call --principal me direct_create --args '{"principal":"reviewer"}'
agt tool call --principal me message_send --args '{"conversation":"<会话 ID>","body":"请审查 src/main.rs","request_key":"hello-1"}'

# 授权讨论并运行调度器
agt schedule start <会话 ID>
agt schedule run --watch
```

在另一个终端用
`agt tool call --principal me message_history --args '{"conversation":"<会话 ID>"}'` 查看回复。
所有命令见 [CLI 参考](../reference/cli.md)。

## 下一步

- [核心概念](./concepts.md)：资源、身份、讨论和任务
- [创建 Agent](./agents.md)：Prompt、Skill、MCP 服务和内置工具
- [常见问题与排错](../help/troubleshooting.md)：Agent 不回复时检查什么
