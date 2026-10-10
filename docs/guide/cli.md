# CLI 参考

最近整理：2026-10-10（从项目 README 拆分）。所有命令都支持全局参数 `--home <DIR>`、`--output human|json|jsonl`，删除命令另有 `--yes`。

## 资源管理

每种资源都支持 `create/list/info/update/delete`（MCP 和 Provider 另外支持
`test`，Provider 还支持 `list-models`；Prompt/Skill 另外支持 `read`）。
`add/ls/show/get/edit/remove` 是对应操作的常用别名。

```bash
# Provider
agt provider create openai \
  --base-url https://api.openai.com/v1 \
  --api-key-env OPENAI_API_KEY
agt provider list
agt provider info default/openai
agt provider update openai --base-url http://localhost:11434/v1
agt provider test openai
agt provider list-models openai

# Model alias
agt model create fast \
  --provider openai \
  --model gpt-4o \
  --api-format chat-completions \
  --tool-call true
agt model update fast --temperature 0.2 --timeout 120
agt model info fast

# Prompt（正文来源三选一）
agt prompt create review --text '请检查代码中的安全问题。'
agt prompt update review --file ./review.md
agt prompt read review

# Skill
agt skill create code-review --description 'Review source code'
agt skill info code-review
agt skill read code-review
agt skill update code-review --file ./code-review/SKILL.md

# MCP（输入必须是单 server mcpServers 文档）
agt mcp create filesystem --file ./filesystem.json
agt mcp info filesystem
agt mcp update filesystem --file ./filesystem.json
agt mcp test filesystem

# Agent
agt agent create coder --model fast --prompt review \
  --skill code-review --mcp filesystem
agt agent update coder --description '主审查 Agent' --model fast
agt agent info coder
```

### 引用与分类

引用可以写成 `name`（使用 `default` category）或
`category/name`。也可以在命令上显式传 `--category <category>`。Category 管理
按资源域分别进行：

```bash
agt model category create team
agt model category list
agt model category delete team
```

删除 category 只允许删除空的非 `default` category。

## 递归删除

删除默认是精确删除。若资源仍被其他定义引用，命令会拒绝并列出引用者：

```bash
agt provider delete openai --home /tmp/agt-home --yes
# provider 仍被引用时：请加 --recursive
```

使用 `--recursive` 会沿着依赖图删除所有依赖该资源的定义，删除顺序为
`agent → model → provider`（其他资源的依赖 Agent 也会先删除）：

```bash
agt provider delete default/openai --recursive --yes
agt model delete fast --recursive --dry-run
```

依赖图只沿入站引用展开：删除 Provider 会级联其 Model 和 Agent；删除 Prompt、
Skill、MCP 或 Harness 会级联引用它们的 Agent。Agent 使用的其他共享资源不会因为 Agent
被删除而反向删除。`--dry-run` 只输出计划，不修改文件。

删除前会扫描全部七类资源并检查悬空引用、损坏配置和受保护对象；任一检查失败
都会 fail-closed。`prompt/default/default` 是不可删除的保留资源。
实际删除先把目录移动到 `<home>/.trash/<timestamp-pid-attempt>`，批量失败时尝试回滚。
非 TTY 环境必须显式提供 `--yes`；TTY 默认会再次确认。

## 修改和按需加载

普通 `update` 修改的是资源本身，引用仍然使用稳定的
`category/name` identity。下次读取或加载引用时会得到最新配置，因此不需要对
所有 Agent 做递归改写。例如修改一个 Model 的温度，引用该 Model 的 Agent 会
自动使用新值。

当前没有 `rename/move` 命令；因此不会伪装成“递归修改”去批量重写引用。若未来
需要修改 identity，将单独实现带冲突检查、引用重写和事务回滚的 rename 操作。

## Agent 内置工具

Web 创建或编辑 Agent 时勾选“框架内置工具”，或用 CLI：

```bash
agt agent create steward --model fast --builtin-tool resource_management
agt agent update coder --add-builtin-tool task_collaboration
agt agent update coder --remove-builtin-tool resource_management
```

外部客户端可通过 `agt serve --principal ID --manager` 使用同一套工具，`agt tool list/call --principal ID --manager` 也一样。
各工具组的权限范围见 [Agent 内置工具](builtin-tools.md)。

## IM

IM 只负责消息和会话本身，不依赖 Agent 或 Provider。参与者可以是用户、
Agent、Bot 或其他服务；托管身份与 Agent 定义的映射由 `agt-service` 管理。

~~~bash
# 创建参与者
agt im principal create alice --name Alice
agt im principal create bob --name Bob

# 私聊（同一对参与者始终复用同一个会话）
agt im direct create alice bob
agt im message send <conversation-id> --sender alice '你好，Bob'

# 建群和管理成员
agt im group create --owner alice --title '项目讨论' --member bob
agt im group members add <conversation-id> carol --actor alice
agt im group members list <conversation-id>

# 查询消息、已读和会话
agt im message list <conversation-id> --viewer bob
agt im message read <conversation-id> --principal bob --through-seq 10
agt im conversation list --principal bob
agt im conversation reactivate <conversation-id> --actor alice
~~~

消息以 SQLite 事务写入，具有会话内单调序号、请求幂等键、游标分页和事件记录。
默认最新消息页通过 `next_cursor` 向旧消息方向翻页（把游标作为 `--before`）；使用
`--after` 时则向新消息方向翻页（把游标继续作为 `--after`）。删除消息或会话采用
软删除/状态标记，归档和删除后的会话可以由仍在会话中的参与者显式 `reactivate`
恢复。IM CLI 不执行 Agent 推理；`agt schedule run` 消费已授权讨论的投递，
`agt serve` 则通过 stdio MCP 开放资源和通信工具。
自动回复的调度规则见[调度器](../architecture/scheduler.md)。新成员加入后可以读取会话已有历史，但未读数从加入时的消息序号开始计算。

事件名称和操作者语义是稳定底座契约：`conversation.created`、`member.joined`、
`member.left`、`member.read`、`message.created`、`message.deleted`、
`conversation.archived`、`conversation.deleted`、`conversation.reactivated`。每个事件
都带有会话内递增 `seq`、可选 `actor_id` 和 JSON `payload`；`actor_id` 表示实际执行
该操作的参与者，而不是被操作的目标。


## 显式 Agent 执行（实验性）

在已有 Provider、Model 和 Agent 定义的 home 中执行：

```bash
agt agent run coder --input '分析这段需求并给出建议' --request-key task-001
echo '请总结这段内容' | agt agent run coder --stdin

# coder 可以选择调用 delegate，把明确任务交给 reviewer，再汇总结果。
# reviewer 需事先通过 agt agent create 创建。
agt agent run coder --input '请委派 reviewer 审查这份方案，然后汇总意见：...' \
  --delegate reviewer --request-key review-001

agt run list --limit 20
agt run info <run-id>
agt run children <run-id>
```

`--home` 和输出参数仍是全局选项。`--cwd` 指定 MCP 工作目录，默认当前目录。
附加到 Agent 的 MCP 会被连接，模型可以调用其工具；持久化 Skill 正文会在执行
开始时加入系统提示词。`--delegate` 是本次执行的允许名单，不保证模型一定委派。
子 Agent 不可继续委派；父子 Run 共享步数统计，上限由 `config.yaml` 的 `scheduler.model_steps` 控制（默认不限），`--max-steps` 仅为兼容保留、不再生效。

每次调用生成独立 Run，状态保存在 `<home>/runtime/runtime.db`。同一 `--request-key`
和相同输入、Agent、允许名单、步数与规范化工作目录会返回原记录，不重新调用模型；
参数不同会冲突。省略 key 时自动生成新 key。失败和取消也是该 key 的终态，重试
必须显式使用新 key。非 `completed` 状态会输出记录并返回非零退出码。

父子 Run 通过 `parent_run_id` 关联。委派双方使用 `runtime:category/name` IM 身份，
私聊只记录委派输入和子 Agent 最终回复，模型/工具的私有上下文不会写入 IM。
同一个定义可用于多个 Run；每次 Run 不自动加载群聊历史或跨 Run 记忆。
开始执行时会加载根 Agent 和全部允许委派对象的配置，并保存指纹；正在运行的
执行不受后续文件修改影响。指纹用于辨认配置，不是可重建完整配置的归档。

显式 `agent run` 的当前边界（自动讨论另见调度设计）：

- 模型协议支持文本 Chat Completions 和 Responses，包括函数工具调用；不接受图片输入。
  Agent 使用明确的 Model 引用，只挂载已入库的 Skill。Skill `allowed-tools` 的执行约束尚未实现，挂载后运行会报错。
- 指定 `reasoning_effort` 时省略 temperature；Chat Completions 使用
  `reasoning_effort` / `max_completion_tokens`，Responses 使用
  `reasoning.effort` / `max_output_tokens`。Responses 以 `store: false` 执行，
  在单次 Run 的内存上下文中回传完整输出（含推理项）与工具结果。具体模型须支持这些参数，运行时不会自动切换协议。
- MCP 结果支持文本与 structuredContent；客户端 `_meta` 不发给模型。
  模型响应上限 4 MiB，请求上下文字节保护随模型窗口扩展（至少 1 MiB）；按 Model 的 context_window
  估算 token 预算，自动裁剪工具输出并压缩历史。窗口配置不限制输出长度。
- Rust API 接受 `CancellationToken`，可在步骤边界取消，MCP 调用支持中途取消。
  HTTP 请求按模型 timeout 超时，期间的取消在请求返回后观察到。CLI 被终止时，
  未完成记录会保留为 `running`；可用 `agt run list` 找回，不会自动重放工具。
- 这是同步、显式执行，不消费 IM 事件。Run 与 IM 分别提交；子执行成功后若回复
  发布失败，父 Run 会失败，子结果仍可查询，当前没有自动补发或恢复执行入口。

会话信息中可重置模型上下文而保留聊天记录；CLI 使用
`agt schedule reset <conversation> --actor <human-principal>`。
自动压缩、跨运行摘要、权限及模块边界见[上下文管理](../architecture/context.md)。


## 外部 Harness 执行

Harness 是可复用资源，Agent 选择 `--model` 或 `--harness`（二选一）。
当前实现本机 Codex app-server，其他引擎尚未注册。

```bash
# local.json 示例：
# {"engine":"codex","executable":"/usr/local/bin/codex",
#  "working_directory":"/absolute/project","sandbox":"read_only"}
agt harness resource create local --file local.json
agt harness resource test local
agt agent create reviewer --harness local --prompt review
agt agent run reviewer --input '审查当前变更'
# agt harness resource 同样支持 list/info/update/delete/category。
```

Web 可在资源库管理 Harness，并在 Agent 编辑页选择外部执行方式。
接入边界、会话恢复与取消语义见 [Harness 设计](../architecture/harness.md)。
