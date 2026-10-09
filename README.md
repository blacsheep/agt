# agt — resources, IM and explicit agent execution

这是从原始 agt runtime 中提取出的本地资源管理器，并提供一个通用的本地
IM 后端。资源管理器和 IM 都可独立使用；`agt-runtime` 支持显式执行、有限委派，
以及由持久化调度器驱动的私聊/群聊 Agent 协作。IM 本身不会自动启动推理；
Web 默认在发送消息后授权回复，CLI 通过 `schedule` 显式授权和调度。

新增框架入口与完整验收步骤见 [框架验收指南](docs/framework-acceptance.md)，
当前架构见 [调度设计](docs/scheduler-plan.md)，全部文档见 [文档导航](docs/README.md)。

想通过界面手动验收？见 [AGT Studio 本地控制台](web/README.md)：构建 CLI 后运行
`node web/server.mjs`，即可配置角色、私聊/群聊、自动回复、暂停/结束讨论及查看状态。

## 前端 + 后端启动

需要 Rust 1.96.1+；Web 需要 Node.js 22+，无需安装前端依赖。

```bash
cargo build -p agt-cli
node web/server.mjs
```

Windows PowerShell 请显式指定可执行文件：

```powershell
cargo build -p agt-cli
node web/server.mjs --binary .\target\debug\agt.exe
```

打开终端打印的完整带令牌链接。Node 提供页面与本机 HTTP 接口，并启动 Rust
调度进程。升级代码后重新构建 CLI、重启 Web 并刷新页面。
远程部署的本机访问方式与安全限制见 [Web 指南](web/README.md)。

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
agt-agent      agent.yaml 及 model/harness/prompt/skill/MCP 引用
agt-harness    harness.yaml 资源、Codex 协议与执行控制
agt-im         通用参与者、私聊/群聊、消息、已读和事件存储（SQLite）
agt-service    资源工具、身份权限、Agent 绑定、响应策略和 stdio MCP 服务
agt-inference  Agent 配置解析、模型协议与流式解析、上下文预算与压缩（无持久化）
agt-runtime    模型/工具循环、持久化 Run、消息投递调度和显式委派
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
  harnesses/default/
  im/                         # 首次使用 agt im 时自动创建
    im.db
  runtime/                    # 按首次使用相应功能创建
    runtime.db                # 托管绑定、讨论与任务、Run 与调用诊断、会话摘要、
                              # Harness 会话、审批和事件投影（按模块分表）
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
Web 自动回复使用持久化并行调度器：不同 Agent 可同时回应，接话时可判断参与、
沉默或等待。消息独立入队，无固定 5 秒等待；Native 支持 steering 和 follow-up，参与判断并行，自主回复依次开始。
每位 Agent 最多开始 100 次回复，新用户消息获准自动回复后计数归零，整轮模型调用次数不设上限。CLI 可用 `agt schedule start <conversation-id>`
授权讨论、`agt schedule run --watch` 持续处理，以及 `pause/resume/end/status`
控制和观察。每个 home 仅一个活跃调度器；具体行为见 [调度设计](docs/scheduler-plan.md)。
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

## Agent 内置资源工具

Web 创建或编辑 Agent 时，勾选“框架内置工具 → 资源管理”，即可让模型操作
当前 home 的资源，无需额外启动 MCP 服务。CLI 同样支持：

```bash
agt agent create steward --model fast --builtin-tool resource_management
agt agent update coder --add-builtin-tool resource_management
agt agent update coder --remove-builtin-tool resource_management
```

默认无资源权限；显式挂载开放六类资源 CRUD、Provider 模型列表、分类管理和
Provider/MCP 静态配置校验。不会同时授予身份和托管管理权限。
删除仍需精确计划确认，模型可自行完成两次调用；挂载意味着授权资源写入和删除。
配置按每次运行快照加载，取消挂载从下一次运行生效，委派不继承父 Agent 权限。

外部客户端仍可通过 `agt serve --principal ID --manager` 使用相同工具；
CLI 的 `agt tool list/call --principal ID --manager` 使用同一套契约。
Provider 写入接受直接 `api_key`，也兼容 `api_key_env`，两者互斥且读取始终脱敏。
详见 [内置资源工具契约](docs/builtin-resource-tools-proposal.md)。

## 显式 Agent 执行（实验性）

在已有 Provider、Model 和 Agent 定义的 home 中执行：

```bash
agt agent run coder --input '分析这段需求并给出建议' --request-key task-001
echo '请总结这段内容' | agt agent run coder --stdin

# coder 可以选择调用 delegate，把明确任务交给 reviewer，再汇总结果。
# reviewer 需事先通过 agt agent create 创建。
agt agent run coder --input '请委派 reviewer 审查这份方案，然后汇总意见：...' \
  --delegate reviewer --request-key review-001 --max-steps 12

agt run list --limit 20
agt run info <run-id>
agt run children <run-id>
```

`--home` 和输出参数仍是全局选项。`--cwd` 指定 MCP 工作目录，默认当前目录。
附加到 Agent 的 MCP 会被连接，模型可以调用其工具；持久化 Skill 正文会在执行
开始时加入系统提示词。`--delegate` 是本次执行的允许名单，不保证模型一定委派。
子 Agent 不可继续委派；所有父子模型调用共享 `--max-steps`（1–100，默认 12）。

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
自动压缩、跨运行摘要、权限及模块边界见 [上下文管理](docs/context-management.md)。

架构、后续里程碑与验收说明见 [runtime plan](docs/runtime-plan.md)。

## 开发与验证

```bash
cargo fmt --all -- --check
cargo check -p agt-cli
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p agt-cli -- --help
```

测试布局约定：

- 简短单元测试写在被测文件内的 `#[cfg(test)] mod tests {}`。
- 需要私有项的较长单元测试放在被测模块目录下：`<module>/tests.rs`，按主题再拆为
  `<module>/tests/<topic>.rs`；一律用普通 `mod` 声明，不使用 `#[path]` 或 `include!`。
- 只经公共 API 的集成测试放在 `crates/<crate>/tests/`：单文件为 `<suite>.rs`，多文件为
  `<suite>/main.rs` 加同目录子模块；集成测试不引用 `src/` 下的文件。

CLI 集成测试覆盖六类资源 CRUD、Provider secret 脱敏、Provider → Model →
Agent 的递归删除，以及 IM 私聊、幂等消息和群成员生命周期。

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
接入边界、会话恢复与取消语义见 [Harness 设计](docs/harness-integration.md)。
