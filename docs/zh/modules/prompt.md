# Prompt

| 概览 | |
| --- | --- |
| **是什么** | 角色描述，作为 Agent 的系统提示词 |
| **CLI** | `agt prompt` |
| **Agent 工具** | `kind: "prompt"` 的 `resource_*` |
| **Web 控制台** | **资源库** → Prompt；Agent 表单中的**角色 Prompt** |
| **文件** | `prompts/<category>/<name>/PROMPT.md` |
| **Crate** | `agt-prompt` |

## 作用

Prompt 定义 Agent 的角色：负责什么、和谁协作、什么时候应该保持沉默。多个 Agent 可以共用一个 Prompt；修改后，它们都从下一次执行开始使用新内容。

agt 自带两个 Prompt：

| Prompt | 用途 |
| --- | --- |
| `default/default` | 最简的通用 Prompt。不能删除。 |
| `default/lead` | 了解整个框架，能组建并运行团队。见[让 lead 组建团队](../guide/lead-team.md)。 |

`agt init` 会创建这两个 Prompt，且从不覆盖已存在的 Prompt。

新版 agt 改进了 `default/lead` 时，只要你从没改过这个 Prompt，下次调度器启动（`agt web` 或 `agt schedule run`）时就会自动换成新版。改过的 lead Prompt 不会被动；这时它在**资源库**里的详情页会提示已有更新的内置版本，并提供**替换为新版内置提示词**按钮，替换会丢弃你的修改。`resource_read` 也会在 `builtin_update.text` 中返回新版正文。

## 怎么用

**Web 控制台：** **资源库** → **新建资源** → Prompt，或在 Agent 表单中选择**新写一份**。Prompt 默认预览前 2,000 个字符，可以展开全文。

**CLI：**

```bash
agt prompt create reviewer --text '你负责审查代码的正确性……'
agt prompt update reviewer --file ./reviewer.md
cat reviewer.md | agt prompt update reviewer --stdin
agt prompt read reviewer
```

**Agent 工具：**

```json
{"kind":"prompt","reference":"reviewer","create":true,"data":{"text":"你负责审查代码……"}}
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `--text` / `--file` / `--stdin` | Prompt 正文的来源（三选一） |

## 全部参数

### `agt prompt create <名称>` 和 `agt prompt update <名称>`

| 选项 | 含义 |
| --- | --- |
| `--text <文本>` | Prompt 正文 |
| `--file <路径>` | 从文件读取正文 |
| `--stdin` | 从标准输入读取正文 |
| `--category <c>` | 分类（默认 `default`） |

`--text`、`--file` 和 `--stdin` 必须且只能给出一个。更新会替换全部正文。

### 其他命令

| 命令 | 含义 |
| --- | --- |
| `read <引用>` | 打印 Prompt 正文 |
| `list`、`info`、`delete`、`category` | 见[资源通用操作](./resources.md#通用-cli-命令) |

### 文件：`PROMPT.md`

纯 Markdown，整个文件就是 Prompt。

### 工具字段（`resource_write`，`kind: "prompt"`）

| 字段 | 含义 |
| --- | --- |
| `text` | 完整的 Prompt 正文 |

## 写好 Prompt

- 说明角色、协作的人和 Agent，以及什么算“完成”。
- 在群聊中，写明什么时候只回复 `[[silent]]`，避免 Agent 对每条消息都发言。
- 写明重要操作应使用的工具，例如需要结果的工作使用 `task_request`。

## 相关模块

- [Agent](./agent.md)：给 Agent 挂载 Prompt
- [Skill](./skill.md)：在 Prompt 之上追加的可复用指令
