# 权限审批

| 概览 | |
| --- | --- |
| **是什么** | 单次工具调用和 Codex 请求的审批 |
| **CLI** | `agt permission` |
| **Web 控制台** | 会话中的执行横条；**交互观察** → **权限审批**；**运行设置** → **工具权限审批** |
| **设置** | `permissions.*` |
| **存储** | `runtime/runtime.db` |
| **Crate** | `agt-runtime` |

## 作用

Native Agent 的每次工具调用（内置工具、MCP 工具或本机命令）在执行前都要经过工作空间审批策略。Codex Agent 需要超出沙箱时会发出自己的审批请求。
两类请求进入同一个审批队列。

| Native 策略 | 行为 |
| --- | --- |
| `model_review`（默认） | 由审核模型批准或拒绝每次调用；不明确的交给人 |
| `manual` | 每次调用都等待人决定 |
| `auto_allow` | 所有调用直接执行 |

默认设置中没有配置审核模型，所以在你选择审核模型或其他策略之前，**每次调用都会交给人处理**。

## 怎么用

**Web 控制台：** 打开**运行设置** → **工具权限审批**，设置 **Native 权限审核策略**、**审核模型**和 **Harness 权限处理模式**。
在会话的执行横条或**交互观察** → **权限审批**中批准或拒绝请求。

**CLI：**

```bash
agt permission list
agt permission info <id>
agt permission decide <id> --expected-revision 1 --decision '"allow"' --reason '常规读取'
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `permissions.mode` | Native 策略 |
| `permissions.reviewer_model` | 用于审核的 Model 资源 |
| `permissions.harness_mode` | Codex 请求的 `manual` 或 `auto_allow` |

## 全部参数

### 设置（`config.yaml`）

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `permissions.mode` | `model_review` | Native 工具调用的 `model_review`、`manual` 或 `auto_allow` |
| `permissions.reviewer_model` | `null` | 一个 Model 资源引用。未设置时，`model_review` 会把每次调用交给人。 |
| `permissions.review_timeout_seconds` | `30` | 审核模型的时限，1 到 120。超时后调用交给人。 |
| `permissions.request_timeout_seconds` | `600` | Native 调用等待任何决定的时长。超时后取消执行。 |
| `permissions.harness_mode` | `manual` | `manual`：每个 Codex 请求等待人决定。`auto_allow`：接受 Codex 的请求，仅限该请求或该轮。 |

修改对正在等待和之后的请求生效。已作出的决定保持不变。

### `agt permission` 命令

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `list` | `--principal <id>`、`--before <游标>` | 审批请求，从新到旧 |
| `info <id>` | `--principal <id>` | 一个请求：工具、参数、原因、选项、revision |
| `decide <id>` | `--expected-revision <n>`（必填）、`--decision <json>`（必填）、`--reason <文本>`、`--principal <id>` | 作出决定 |

`--decision` 是 `info` 中某个选项的 JSON `value`：

| 来源 | 选项 |
| --- | --- |
| Native 调用 | `"allow"`、`"deny"`、`"cancel"`（取消执行） |
| Codex 命令或文件修改 | 批准本次、批准本会话、拒绝、取消本轮（以 Codex 提供的为准） |
| Codex 权限请求 | 授予本轮、拒绝、取消本轮 |

### 模型审核如何工作

审核模型收到工具名、参数以及 Agent 正在处理的请求，不使用任何工具，只回答一次。审核模型的拒绝是最终结果。
错误、超时、无效输出和不确定的回答都会交给人。人可以在审核模型回答之前作出决定。

### 审批不能做什么

- 给 Agent 它没有的工具，或绕过成员资格和讨论限制。
- 撤回已被批准并发出的调用。
- 改变 Codex 的沙箱。

## 相关模块

- [Agent](./agent.md)：Agent 有哪些工具
- [Model](./model.md)：审核者是普通的 Model 资源
- [Harness](./harness.md)：Codex 请求的来源
