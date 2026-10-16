# 权限审批

给 Agent 挂载工具，决定了它*能*调用什么；审批决定某一次调用*是否可以*执行。agt 有两条审批路径：一条处理 Native Agent 的工具调用，一条处理 Codex 发出的请求。

## Native 工具调用

Native Agent 执行工具之前，调用要经过工作空间策略 `permissions.mode`。这包括 MCP 工具、本机命令，以及会修改数据或访问 agt 外部的内置工具。只读取 agt 自身数据的内置工具，例如 `message_history`、`task_list`、`memory_read` 和 `resource_list`，在任何模式下都不需要审批，因为它们只能看到 Agent 本来就有权看到的内容。

| 模式 | Web 标签 | 行为 |
| --- | --- | --- |
| `model_review`（默认） | 模型审核，异常转人工 | 由审核模型批准或拒绝每次调用。没有设置审核模型，或审核失败、超时、结论不明确时，转给你处理。 |
| `manual` | 人工审批 | 每次调用都等待你的决定。 |
| `auto_allow` | 自动放行 | 所有调用都不经审核直接执行。 |

::: tip 默认会询问你
默认模式是 `model_review`，但初始状态没有配置审核模型，所以在你选择审核模型之前，所有需要审批的调用都会转为人工审批。
:::

审核模型是一个普通的 Model 资源，独立于被审核的 Agent。它会看到工具名、参数以及 Agent 正在处理的请求，并给出一次允许或拒绝的结论。审核模型的拒绝是最终结果；其他情况都会转给你。

被拒绝的调用不会执行。Agent 会被告知调用被拒绝，然后继续工作。参数无效的调用在进入审批队列之前就会被拒绝。

MCP 工具总是需要审批，除非你信任该服务的只读标注，见 [MCP](../modules/mcp.md#工具调用如何检查)。

### 总是允许某个 Agent 使用某个工具

批准调用时，可以选择**总是允许此 Agent 使用该工具**。之后这个 Agent 再调用这个工具时，不管哪种模式都不再询问，审批记录里的决定来源显示为"已保存的总是允许规则"。一条规则只针对一个 Agent 和一个工具，参数不限。

在**权限审批**面板顶部可以查看和移除规则，也可以用 CLI：

```bash
agt permission rules
agt permission forget --agent default/lead --tool framework_resource_write
```

## Codex 审批

当命令、文件修改或额外权限超出 Codex 沙箱时，Codex 会自己请求审批。这些请求使用 `permissions.harness_mode`：

| 模式 | 行为 |
| --- | --- |
| `manual`（默认） | 每个请求都等待你的决定。可以批准本次、批准本会话、拒绝或取消本轮。 |
| `auto_allow` | agt 接受 Codex 的请求，仅限该请求或该轮。 |

两种模式都不会改变 Codex 的沙箱设置。

## 在哪里审批

- **在会话中。** 有请求等待时，输入框上方的执行横条会自动展开。点击可查看工具、参数和原因，然后批准或拒绝。
- **交互观察** → **权限审批**：整个工作空间的所有请求，支持筛选。
- **CLI：**

```bash
agt permission list
agt permission info <id>
agt permission decide <id> --expected-revision <rev> --decision '"allow"' --reason '没问题'
```

`--decision` 接受 `permission info` 列出的某个选项的 JSON `value`，例如 Native 调用的 `"allow"`、`"allow_always"`、`"deny"` 或 `"cancel"`。

在 `permissions.request_timeout_seconds`（默认 600 秒）内没有得到决定的请求会取消这次执行。

## 修改策略

在 Web 控制台中，于**运行设置** → **工具权限审批**或审批面板中设置模式和审核模型。也可以写在文件里：

```yaml
permissions:
  mode: model_review
  reviewer_model: default/reviewer
  review_timeout_seconds: 30
  request_timeout_seconds: 600
  harness_mode: manual
```

修改对正在等待和之后的所有请求生效。已作出的决定不会撤回。

## 审批不覆盖的范围

- 审批不会给 Agent 它没有的工具，也不能绕过成员资格或讨论限制。
- `resource_management` 的删除使用自己的两步确认（先获取计划，再确认完整列表），由 Agent 自己完成。批准这次工具调用就是批准删除。
- 调用一旦被批准并发往 MCP 服务或 Shell，就无法撤回。
