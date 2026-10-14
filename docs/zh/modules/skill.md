# Skill

| 概览 | |
| --- | --- |
| **是什么** | 加入 Agent 系统提示词的可复用指令包 |
| **CLI** | `agt skill` |
| **Agent 工具** | `kind: "skill"` 的 `resource_*` |
| **Web 控制台** | **资源库** → Skill；Agent 表单中的 Skills |
| **文件** | `skills/<category>/<package>/SKILL.md` |
| **Crate** | `agt-skill` |

## 作用

Skill 把某一类工作的指令打包，例如“我们如何审查代码”或“如何执行发布检查清单”，这样可以挂载到任何 Agent 上，而不必复制到每个 Prompt 里。执行开始时，所有挂载的 Skill 的完整正文会加入 Agent 的指令。Codex Agent 也会收到 Skill。

Skill 使用 `SKILL.md` 格式：带 YAML front matter 的 Markdown。包名可以带作用域，例如 `@team/review`。

## 怎么用

**Web 控制台：** **资源库** → **新建资源** → Skill 会创建只含名称和描述的骨架。完整指令请用 CLI 编写。

**CLI：**

```bash
agt skill create code-review --description 'Review source code for correctness and style'
agt skill update code-review --file ./code-review/SKILL.md
agt skill read code-review
agt agent update reviewer --add-skill code-review
```

**Agent 工具：**

```json
{"kind":"skill","reference":"code-review","create":true,"data":{"description":"Review source code"}}
{"kind":"skill","reference":"code-review","create":false,"data":{"text":"---\nname: code-review\n..."}}
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `--description` | 一行摘要，显示在列表中 |
| `--file` | 一个 `SKILL.md` 文件，或包含它的目录 |

## 全部参数

### `agt skill create <包名>`

| 选项 | 必填 | 含义 |
| --- | --- | --- |
| `--description <文本>` | 是 | 摘要，最多 1024 个字符 |
| `--category <c>` | 否 | 分类（默认 `default`） |

创建一个包含名称、描述和占位正文的 `SKILL.md`。

### `agt skill update <包名>`

| 选项 | 必填 | 含义 |
| --- | --- | --- |
| `--file <路径>` | 是 | `SKILL.md` 文件或包含它的目录。替换整个文件。 |
| `--category <c>` | 否 | 分类 |

### 其他命令

| 命令 | 含义 |
| --- | --- |
| `read <引用>` | 打印 `SKILL.md` |
| `list`、`info`、`delete`、`category` | 见[资源通用操作](./resources.md#通用-cli-命令) |

### 文件：`SKILL.md`

```markdown
---
name: code-review
description: Review source code for correctness and style
license: MIT
---
# 代码审查

1. 评论前先读完整个改动。
2. ……
```

| Front matter 字段 | 必填 | 含义 |
| --- | --- | --- |
| `name` | 是 | 包名；必须与目录名一致 |
| `description` | 是 | 摘要，最多 1024 个字符 |
| `license` | 否 | 指令的许可证 |
| `compatibility` | 否 | 兼容性说明，最多 500 个字符 |
| `metadata` | 否 | 字符串映射。带作用域的包用 `agt.package` 保存完整名称，例如 `@team/review`，其最后一段必须等于 `name`。 |
| `allowed-tools` | 否 | 暂不支持：挂载了此类 Skill 的 Agent 会运行失败 |

### 工具字段（`resource_write`，`kind: "skill"`）

| 字段 | 时机 | 含义 |
| --- | --- | --- |
| `description` | 创建 | 摘要；创建骨架 |
| `text` | 更新 | 新的完整 `SKILL.md`，包括 front matter |

## 相关模块

- [Agent](./agent.md)：挂载 Skill
- [Prompt](./prompt.md)：基础角色描述
