# 创建 Agent

Agent 定义把执行引擎、角色和能力组合在一起：

```yaml
# agents/default/reviewer/agent.yaml
name: reviewer
description: 审查代码改动
model: { category: default, name: fast }   # 要么是 Model……
harness: null                              # ……要么是 Harness，二选一
prompt: { category: default, name: reviewer }
skills: [code-review]
mcps:
  - { category: default, name: github }
builtin_tools: [task_collaboration]
```

通常不需要手动编辑这个文件：Web 控制台、CLI 和拥有 `resource_management` 的 Agent 都经过同一套校验写入它。

## 在 Web 控制台中

打开 **Agent 团队**，点击**创建角色**。表单包含这些部分：

| 字段 | 含义 |
| --- | --- |
| **资源名称 / 身份 ID** | 定义名称，同时也是 Agent 的聊天身份 |
| **执行方式** | **AGT 内置**（选择模型）或**外部 Harness**（Codex，选择 Harness） |
| **角色 Prompt** | **使用已有 Prompt**（选择已有 Prompt）或**新写一份** |
| Skills / MCP | 可选，挂载的 Skill 和 MCP 服务 |
| **框架内置工具** | **资源管理**、**任务协作**、**本机命令** |
| **响应策略** | Agent 何时回复；见[对话与自动回复](./conversations.md#响应策略) |

保存会创建定义、身份和绑定，Agent 即可对话。如果某一步失败，表单会显示哪些步骤已完成；修正问题后再次保存即可完成剩余步骤。

之后要修改 Agent，打开它的卡片使用**编辑角色**。修改从 Agent 的下一次执行开始生效。启用、停用和响应策略在同一张卡片的**配置与管理**中。

## Prompt

Prompt 是 Agent 的角色：负责什么、和谁协作、什么时候应该保持沉默。agt 自带两个 Prompt：

- `default/default`：最简的通用 Prompt。
- `default/lead`：了解整个框架、能组建和运行团队的 Prompt。见[让 lead 组建团队](./lead-team.md)。

```bash
agt prompt create reviewer --text '你负责审查代码的正确性……'
agt prompt update reviewer --file ./reviewer.md
agt prompt read reviewer
```

适合群聊协作的 Prompt 会写明什么时候回复 `[[silent]]`，避免 Agent 对群里每条消息都发言。

## Skill

Skill 是可复用的指令包（`SKILL.md`，front matter 中包含名称和描述）。执行开始时，所有挂载的 Skill 正文会加入 Agent 的系统提示词。

```bash
agt skill create code-review --description 'Review source code'   # 创建骨架
agt skill update code-review --file ./code-review/SKILL.md         # 替换正文
```

暂不支持声明了 `allowed-tools` 的 Skill；挂载这类 Skill 会导致执行失败。

## MCP 服务

挂载 MCP 服务可以给 Agent 提供工具。Agent 在每次执行开始时连接这些服务，并可调用它们列出的工具。见 [MCP 工具](./mcp.md)。

```bash
agt agent update reviewer --add-mcp github
agt agent update reviewer --remove-mcp github
```

## 内置能力

| 能力 | Web 标签 | 赋予 Agent 的能力 |
| --- | --- | --- |
| `resource_management` | 资源管理 | 管理 home 中的所有资源，创建 Agent 身份，绑定并启用 Agent |
| `task_collaboration` | 任务协作 | 委派可追踪任务，等待并汇报结果 |
| `local_execution` | 本机命令 | 在宿主机执行 Shell 命令（仅 Native Agent；还需开启 `execution.enabled`） |

```bash
agt agent create steward --model fast --builtin-tool resource_management
agt agent update reviewer --add-builtin-tool task_collaboration
```

`resource_management` 权限很大：它覆盖 home 中的所有资源，包括其他 Agent 的定义。只把它交给你信任其管理工作空间的
Agent，例如 lead。详见 [Agent 模块](../modules/agent.md#内置能力)。

## 在 CLI 中启用 Agent

`agt agent create` 只写入定义。要让它参与对话，还需创建身份并绑定：

```bash
agt agent create reviewer --model fast --prompt reviewer
agt tool call --principal me --manager principal_create \
  --args '{"id":"reviewer","name":"Reviewer","kind":"agent"}'
agt tool call --principal me --manager agent_bind \
  --args '{"principal":"reviewer","agent":"reviewer"}'
```

`agent_bind` 可选参数有 `working_directory`（默认是 home）和 `policy`（默认 `mentioned`）。
用 `agent_set_enabled` 可以停用和重新启用 Agent，而不丢失其设置。

## 单次运行 Agent

在会话之外运行一次 Agent，可用于脚本或测试：

```bash
agt agent run reviewer --input '总结这个方案中的风险：……'
echo '总结这段内容' | agt agent run reviewer --stdin
```

这次执行和其他执行一样会被记录（`agt run list`）。传入 `--request-key` 后，重试会直接返回之前的结果，而不会再次调用模型。

## 删除

```bash
agt agent delete reviewer --dry-run      # 查看将删除哪些内容
agt agent delete reviewer --yes
```

仍有绑定的 Agent 需要先解除绑定（卡片上的**运行设置**，或 `agent_unbind`）。Provider、Model、Prompt、Skill 或 MCP
服务仍被 Agent 使用时，删除会被拒绝，除非传入 `--recursive`，它会一并删除依赖它的 Agent。
被删除的资源目录会移动到 `<home>/.trash/`。
