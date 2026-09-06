# agt resource manager

这是从原始 agt runtime 中提取出的本地资源管理器，并提供一个通用的本地
IM 后端。资源管理器和 IM 都可独立使用；IM 不会自动启动 agent runtime、
Gateway 或 session。

## 保留的模块

```text
agt-core       通用 ID、category、typed reference
agt-storage    文件系统基础操作
agt-config     home、目录和 category 初始化
agt-provider   OpenAI-compatible provider
agt-model      model alias 及 provider 引用
agt-prompt     PROMPT.md
agt-skill      SKILL.md（支持 scoped package）
agt-mcp        mcp.json（stdio/http 配置）
agt-agent      agent.yaml 及 model/prompt/skill/MCP 挂载
agt-im         通用参与者、私聊/群聊、消息、已读和事件存储（SQLite）
agt-cli        agt 命令行
```

Agent 配置中不再包含知识库（KB）或 runtime 依赖。所有引用都保存为
`category/name`（Skill 使用 `package` 或 `@scope/package`），资源在读取/运行
时按引用加载。

## 初始化

安装后首次运行 CLI 时，默认 `~/.agt` 会自动准备好，日常使用不需要手动初始化。
只有在为另一个目录建立独立资源 home 时才需要调用 `init`，并且必须显式传入目标目录：

```bash
agt init ~/my_agent_project
# 在源码工作区中也可以这样运行：
cargo run -q -p agt-cli -- init /tmp/agt-home
```

`agt init` 不带目标目录会失败；如果同时传入全局 `--home`，它必须与位置参数目录一致。

初始化后的定义目录：

```text
<home>/
  config.yaml
  providers/default/
  models/default/
  prompts/default/default/PROMPT.md
  skills/default/
  mcps/default/
  agents/default/
  im/                         # 首次使用 agt im 时自动创建
    im.db
```

资源 home 及其内部的 domain、category、resource 目录/配置文件不接受
symlink；这样即使目录中出现指向 home 外部的链接，也不会被读取或写入。home
上层的系统路径链接（例如 macOS 的 `/var`）不在检查范围内。

所有命令都支持以下全局参数：

```text
--home <DIR>                 指定资源 home，默认 $HOME/.agt（`init` 使用位置参数）
--output human|json|jsonl    选择输出格式
--yes                        跳过删除确认（仅删除命令）
```

## CLI

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

## 通用 IM

IM 第一版只负责消息和会话本身，不绑定 Agent 或 Provider。参与者可以是用户、
Agent、Bot 或其他服务；后续运行时只需把自己的身份映射为一个 participant 即可。

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
恢复。当前 IM CLI 不执行 Agent 推理，也不负责网络服务。
新成员加入后可以读取会话已有历史；但未读数从加入时的消息序号开始计算。

事件名称和操作者语义是稳定底座契约：`conversation.created`、`member.joined`、
`member.left`、`member.read`、`message.created`、`message.deleted`、
`conversation.archived`、`conversation.deleted`、`conversation.reactivated`。每个事件
都带有会话内递增 `seq`、可选 `actor_id` 和 JSON `payload`；`actor_id` 表示实际执行
该操作的参与者，而不是被操作的目标。

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
Skill 或 MCP 会级联引用它们的 Agent。Agent 使用的其他共享资源不会因为 Agent
被删除而反向删除。`--dry-run` 只输出计划，不修改文件。

删除前会扫描全部六类资源并检查悬空引用、损坏配置和受保护对象；任一检查失败
都会 fail-closed。`prompt/default/default` 与系统 Gateway Agent 永远不能删除。
实际删除先把目录移动到 `<home>/.trash/<timestamp-pid-attempt>`，批量失败时尝试回滚。
非 TTY 环境必须显式提供 `--yes`；TTY 默认会再次确认。

## 修改和按需加载

普通 `update` 修改的是资源本身，引用仍然使用稳定的
`category/name` identity。下次读取或加载引用时会得到最新配置，因此不需要对
所有 Agent 做递归改写。例如修改一个 Model 的温度，引用该 Model 的 Agent 会
自动使用新值。

当前没有 `rename/move` 命令；因此不会伪装成“递归修改”去批量重写引用。若未来
需要修改 identity，将单独实现带冲突检查、引用重写和事务回滚的 rename 操作。

## 开发与验证

```bash
cargo fmt --all -- --check
cargo check -p agt-cli
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p agt-cli -- --help
```

CLI 集成测试覆盖六类资源 CRUD、Provider secret 脱敏、Provider → Model →
Agent 的递归删除，以及 IM 私聊、幂等消息和群成员生命周期。
