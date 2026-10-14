# 让 lead 组建团队

每个 Agent 都可以由你亲手创建，但通常更省事的做法是给一个 Agent 资源管理能力，让它组建目标所需要的团队。这个 Agent 就是 lead。

## lead 是什么

lead 是一个普通 Agent，只是具备三样东西：

1. 内置 Prompt `default/lead`；
2. `resource_management` 能力，用来创建 Prompt、Agent 和工具；
3. `task_collaboration` 能力，用来委派可追踪的工作并收集结果。

`default/lead` Prompt（由 `agt init` 和首次启动时创建）教 Agent：

- **先查后建**：列出已有的模型、Prompt、MCP 服务和 Agent，优先复用；
- **组建可用的队友**：编写角色 Prompt、定义 Agent、创建身份、绑定、确认已启用，再把它加入会话；
- **最小授权**：新 Agent 不继承 lead 的能力，lead 只给每个角色它需要的能力；
- **让 Codex 造缺少的工具**：把 MCP 服务交给 Codex Agent 编写并附带契约测试，然后注册、探测，再挂载给需要的 Agent；
- **委派与跟进**：需要结果的工作用可追踪任务，并向原会话汇报；
- **让团队自己运转**：一次讲清长期规则（谁负责什么、谁直接找谁、什么时候上报），之后不再逐条转述；
- **正确加入群聊**：在别人建的群里 lead 只是普通成员，不能拉人，它会请群主把它设为管理员，或请群主代为拉人；
- **如实汇报**：在你提问的会话里确认安排、交付结果；每个结论都基于工具的实际返回；删除、覆盖他人配置或开销较大的决定先征求你的同意。

## 设置

按照[快速开始](./quick-start.md#_3-创建-lead)操作，或使用 CLI：

```bash
agt agent create lead --model fast --prompt lead \
  --builtin-tool resource_management --builtin-tool task_collaboration
agt tool call --principal me --manager principal_create --args '{"id":"lead","name":"Lead","kind":"agent"}'
agt tool call --principal me --manager agent_bind --args '{"principal":"lead","agent":"lead"}'
```

lead 会进行大量工具调用。开始前请设置一个你能接受的审批策略，见[权限审批](./permissions.md)。 lead 适合使用工具调用可靠的强模型；队友可以使用更便宜的模型。

### 可选：用于造工具的 Codex Agent

要让 lead 能造工具，先创建一个它可以委派的 Codex Agent：

1. 创建一个 [Harness](./codex.md)，指向一个专门存放工具的工作目录，权限为 `workspace_write`。
2. 基于这个 Harness 创建一个 Agent，例如 `toolsmith`，配一个简短的 Prompt，例如“你在这个目录中构建并测试 MCP 服务”。

告诉 lead 它的存在，或让 lead 用 `resource_list` 自己找到它。

## 给它一个目标

在私聊中和 lead 对话。说出你想达成的结果就行，不用告诉它怎么配置 agt。好的目标会说明结果、你对角色的想法（如果有），以及你希望如何参与：

> 为“我们是否应该迁移到 Postgres 17？”这个问题组建一个三人研究小组：一人研究性能，一人研究运维风险，再加一个挑战双方观点的反方。把他们和我拉进一个群，让他们讨论，最后给我一页纸的总结。

> 我们想玩剧本杀。写一个 MCP 服务保存秘密线索，并且只把每个玩家自己的线索发给他。创建一个主持人和四个玩家 Agent，挂载这个工具，然后在有我的群里开始游戏。

## 会发生什么

- lead 先列出已有资源，再创建 Prompt、Agent 和身份。每一步都是一次工具调用，你可以在执行横条中看到；如果使用人工审批，也在那里批准。
- 新 Agent 一创建就会出现在 **Agent 团队**中。
- lead 会建一个群，把你设为群主，并发布任务说明。此后团队成员之间直接通过提及对话。
- lead 委派可追踪任务时，可以在会话标题栏的**任务**中跟踪。
- lead 会在你们的私聊中汇报。

lead 创建的一切都是普通资源。你可以像对待自己创建的资源一样查看、修改或删除它们。

## 建议

- **检查第一支团队。** 打开每个新 Agent，检查它的 Prompt 和能力。如果某个角色不对，告诉 lead 怎么改，它会自己修改资源。
- **控制成本。** 默认每个 Agent 在一轮讨论中最多回复 100 次，讨论 30 分钟后过期。随时可以用**停止本群 Agent**或 `/stop` 停止一切。长时间任务的设置见[调度器](../modules/scheduler.md#设置-config-yaml)。
- **用独立的 home 做实验。** `agt init ~/experiments/game` 会创建一个干净的工作空间， `agt --home ~/experiments/game web --port 4318` 为它启动一个控制台。
- **写你自己的 lead。** `default/lead` 只是起点。把它复制成新的 Prompt 并针对你的领域修改，例如让 lead 在每个团队中都配一个审查者。
