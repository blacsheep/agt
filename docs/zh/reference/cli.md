# CLI 参考

agt 的所有功能都可以通过 `agt` 命令使用。运行 `agt <命令> --help` 可以查看任意命令的完整选项。

## 全局选项

| 选项 | 含义 |
| --- | --- |
| `--home <目录>` | 仅本条命令使用这个 home（默认 `$AGT_HOME`，否则 `~/.agt`） |
| `--output human\|json\|jsonl` | 输出格式（默认 `human`） |
| `--yes` | 跳过破坏性命令的确认提示（非终端环境下必须提供） |

## 命令一览

| 命令 | 用途 |
| --- | --- |
| `init` | 初始化新的 home 目录 |
| `provider`、`model`、`prompt`、`skill`、`mcp`、`agent` | 管理资源 |
| `harness` | 管理 Codex Harness 及其会话 |
| `im` | 身份、会话和消息 |
| `schedule` | 运行调度器、控制讨论 |
| `tool`、`serve` | 直接调用 agt 的工具，或通过 MCP 开放它们 |
| `run` | 查看 Agent 执行 |
| `permission` | 查看和决定审批请求 |
| `config` | 查看和修改工作空间设置 |
| `events` | 以 JSON 行的形式监听变化 |

## 资源

所有资源命令结构相同：`create`、`list`、`info`、`update`、`delete` 和 `category`。也可以使用别名 `add`、`ls`、
`show`/`get`、`edit` 和 `remove`。资源用 `name`（`default` 分类中）或 `category/name` 引用，也可以用 `--category` 指定。

### provider

```bash
agt provider create <名称> --base-url <url> (--api-key <密钥> | --api-key-stdin | --api-key-env <变量名>)
agt provider update <名称> [--base-url <url>] [--api-key ... ]
agt provider test <名称>          # 校验配置
agt provider list-models <名称>   # 调用 Provider 的 /models 接口
```

### model

```bash
agt model create <名称> --provider <provider> --model <模型 ID> \
  [--api-format chat-completions|responses] [--tool-call true|false] \
  [--temperature <t>] [--reasoning-effort <档位>] [--max-tokens <n>] \
  [--timeout <秒>] [--context-window <tokens>]
agt model update <名称> [相同选项]
```

默认值见[接入模型](../guide/models.md#model)。

### prompt

```bash
agt prompt create <名称> (--text <文本> | --file <路径> | --stdin)
agt prompt update <名称> (--text ... | --file ... | --stdin)
agt prompt read <名称>
```

### skill

```bash
agt skill create <名称> --description <文本>     # 创建 SKILL.md 骨架
agt skill update <名称> --file <SKILL.md 或其所在目录>
agt skill read <名称>
```

### mcp

```bash
agt mcp create <名称> --file <mcp.json>
agt mcp update <名称> --file <mcp.json>
agt mcp test <名称>      # 只校验配置
agt mcp probe <名称>     # 启动/连接、列出工具、检查 schema
```

文件格式见 [MCP 工具](../guide/mcp.md#注册服务)。

### agent

```bash
agt agent create <名称> (--model <model> | --harness <harness>) [--prompt <prompt>] \
  [--skill <skill>]... [--mcp <mcp>]... [--builtin-tool <能力>]... [--description <文本>]
agt agent update <名称> [--model ... | --harness ...] [--prompt ...] [--description ...] \
  [--add-skill/--remove-skill <skill>] [--add-mcp/--remove-mcp <mcp>] \
  [--add-builtin-tool/--remove-builtin-tool <能力>]
agt agent run <名称> (--input <文本> | --stdin) [--request-key <key>] [--cwd <目录>] [--delegate <agent>]...
```

能力包括 `resource_management`、`task_collaboration` 和 `local_execution`。`agent run` 在会话之外运行一次 Agent；
传入 `--delegate` 时，它可以把工作交给列出的 Agent。

### 删除资源

```bash
agt <资源> delete <名称> [--dry-run] [--recursive] [--yes]
```

仍被其他资源使用的资源无法删除，除非传入 `--recursive`，它会一并删除依赖它的 Agent（删除 Provider 时还包括其 Model）。
`--dry-run` 只打印计划，不做任何修改。被删除的目录会移动到 `<home>/.trash/`。`default/default` Prompt 不能删除，
Agent 需要先解除绑定才能删除。

### 分类

```bash
agt <资源> category create <分类>
agt <资源> category list
agt <资源> category delete <分类>    # 只能删除空的非 default 分类
```

## harness

```bash
agt harness resource create <名称> --file <harness.json>
agt harness resource update <名称> --file <patch.json>
agt harness resource test <名称>
agt harness models <名称>          # Codex 报告的模型和推理档位
agt harness skills <名称>          # 项目中的 Codex Skill
agt harness mcp <名称>             # Codex 的 MCP 服务
agt harness capabilities           # agt 支持的引擎操作

agt harness session list
agt harness session current --principal <agent> --conversation <id>
agt harness session configure <会话> --model <模型> [--effort <档位>]
agt harness session rename <会话> <标题>
agt harness session compact <会话>
agt harness session cancel <会话>
```

文件格式见 [Codex Agent](../guide/codex.md#创建-harness)。

## im

```bash
agt im principal create <id> --name <名称> [--kind user|agent|...]
agt im principal list
agt im owner get | ensure <用户>         # Agent 建群时自动加入的工作空间负责人

agt im direct create <a> <b>
agt im group create --owner <id> --title <标题> --member <id>...
agt im group members add|remove|leave|list <会话> ...
agt im group dissolve <会话> --actor <群主>

agt im conversation list --principal <id>
agt im conversation info|hide|unhide|archive|reactivate|delete <会话> ...

agt im message send <会话> --sender <id> <正文>
agt im message list <会话> --viewer <id>
agt im message read <会话> --principal <id> --through-seq <n>
```

这些命令只保存消息，从不运行 Agent。运行 Agent 请使用 `schedule`。

## schedule

```bash
agt schedule run [--watch] [--concurrency <n>]   # 调度器；不加 --watch 时处理完当前可运行的工作就退出
agt schedule start <会话> [--message <id>]
agt schedule pause <会话>
agt schedule resume <会话>
agt schedule end <会话>
agt schedule reset <会话> --actor <你>      # 重置 Agent 上下文，保留历史
agt schedule delete <会话> --actor <你>
agt schedule dissolve <会话> --actor <群主>
agt schedule status
```

`start` 授权一轮讨论：你待处理的消息（或只是 `--message` 指定的那条）可以被回复了。授权本身不调用模型，
正在运行的调度器才会调用。Web 控制台会替你运行调度器；每个 home 只运行一个调度器。

## tool 和 serve

```bash
agt tool list --principal <id> [--manager]
agt tool call --principal <id> [--manager] <工具> --args '<json>'
agt serve --principal <id> [--manager]          # MCP stdio 服务
```

`--principal` 是要使用的身份；`--manager` 增加资源和绑定工具。见[内置工具](./builtin-tools.md)和
[在 MCP 客户端中使用 agt](../guide/mcp-clients.md)。

## run

```bash
agt run list [--limit <n>] [--conversation <id>]
agt run info <执行>
agt run calls <执行> [--after <id>] [--limit <n>]   # 模型和工具调用记录，不含私有内容
agt run children <执行>                             # 这次执行委派出的执行
agt run tool-detail <执行> --activity <id>          # 某次工具调用的参数和结果
```

## permission

```bash
agt permission list [--principal <id>]
agt permission info <id>
agt permission decide <id> --expected-revision <rev> --decision '<json 值>' [--reason <文本>]
```

## config

```bash
agt config describe              # 每项设置及其默认值、生效时机
agt config show                  # 当前生效的设置
agt config apply ./config.yaml   # 校验并替换整个文件
```

见[运行配置](./configuration.md)。

## events

```bash
agt events watch --principal <id> [--conversation <id>] [--after <游标>]
```

该身份可见的每个变化输出一行 JSON。重启后可以从最后一个 `cursor` 继续。
