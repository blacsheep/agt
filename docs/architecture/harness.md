# Harness：外部执行引擎

最近整理：2026-10-10。本文合并原“Harness 资源与外部执行引擎”“Harness 执行流程”“Harness 执行状态与租约”三篇；
Lead 通过工具管理 Harness 的用法见[内置工具](../guide/builtin-tools.md#harness-管理工具)。

## 决策

Harness 是正式的可复用配置资源；Codex 是执行引擎；Agent 是独立身份与角色。
一个 Agent 选择内置 Model 或外部 harness，二者互斥。
Harness 资源不是群成员，外部框架的内部子 Agent 不会自动成为 AGT 联系人。

## 模块与边界

- `agt-harness`：Harness 资源配置和安全文件存储、Codex app-server 协议适配、进程生命周期。
- `agt-agent`：model/harness 互斥引用、Prompt/Skill/内置工具挂载。
- `agt-service`：Harness CRUD、删除引用检查、统一资源工具契约。
- `agt-runtime/harness.rs`：整项任务执行、Run、外部会话映射、历史增量、取消围栏。
- `agt-runtime/scheduler.rs`：执行调度、群聊提及规则和唯一结果发布。
- CLI/Web：资源与 Agent 配置入口、无推理连接检查和流式预览。
- `agt-im`：保持与引擎无关。

外部执行不实现 ModelClient：完整工具循环只归 Codex 所有，不套用 AGT 的模型重试、
参与判断或摘要模型调用。调度器仅在私聊或明确提及时运行外部 Agent，manual 仍有效。

## 首个接入：本机 Codex

使用 `codex app-server --listen stdio://`。initialize / initialized 后通过
thread/start 或 thread/resume 建立线程，turn/start 启动任务。
item/agentMessage/delta 更新有界预览，item/completed 的非 commentary 消息作为最终回复，
turn/completed 确认最终状态。错误与空最终回复不会自动重跑。

协议依据：<https://developers.openai.com/codex/app-server/>；本机 codex-cli 0.154.0
的 `app-server generate-json-schema --experimental` 用于核实字段与枚举。
动态工具属于实验接口，连接成功不等于所有版本兼容，也不等于模型认证可用。

Harness 配置：engine=codex、服务器绝对 executable、working_directory、可选 model、
sandbox=read_only/workspace_write、timeout_seconds（默认 1800）。不保存 API Key，
沿用运行账户的 Codex 认证与配置。工作区文件可能共享，协作开发应明确分配目录/worktree。
审批使用 Codex 原生 `on-request` + `user`：引擎发出的人工审批请求进入 AGT 审批队列，
决定回传原 JSON-RPC 请求，详见[权限审批](permissions.md)。用户输入类请求仍不支持，遇到时停止并说明原因。
Harness 写入需要 Manager 或挂载 `resource_management` 的 Agent。
AGT 工具权限不构成外部终端的文件隔离；实际文件边界由 Codex sandbox 与操作系统控制。
本轮在 Linux 验证进程组清理及 SIGINT/SIGTERM 取消；Windows 的完整进程树终止尚未验收。

Prompt 和 Skill 正文作为 developerInstructions 注入；allowed-tools 约束未实现时拒绝运行。
框架工具通过 Codex dynamicTools 桥接，逐次使用 AGT 固定身份校验并检查取消围栏。
message_send 可联系其他会话；向当前会话发送时拒绝，由调度器唯一发布最终答复，避免重复消息。
AGT 挂载 MCP 和显式 --delegate allowlist 首版不支持，配置/执行时明确拒绝。
Codex 自身配置中的工具仍由 Codex 管理，其权限和认证不由 AGT 冒充控制。

## 资源与执行会话

资源使用 `harnesses/<category>/<name>/harness.yaml` 文件及通用 CRUD，CLI 入口为
`agt harness resource …`，工具资源类型为 `harness`，Agent 通过 `harness` 字段引用。
旧 environment/env 命令、environment 资源类型、Agent 字段和 environments 目录不再支持，
不提供别名或自动迁移。资源包括项目目录、说明、默认模型和默认 reasoning_effort。
引擎适配器封装 app-server；AGT 会话记录与 Codex session_id 分离，thread ID 只取自引擎响应。

`harness_sessions` 按 Agent 配置、principal、conversation、reset cutoff 标识 AGT 会话记录。
首次准备记录时保存 Harness 配置快照；首次执行时固定引擎返回的默认模型/档位。更新资源默认值不影响已有记录；/new 后
新记录采用新默认值。修改 Agent 的角色/工具配置会另建记录，避免旧线程保留失效权限。
执行会话使用新表，不加载或迁移旧格式会话；引擎自行保存的线程不由资源重命名操作删除。
同一目录中的文件与运行账户的 Codex 全局配置仍共享；快照不是文件系统或账户隔离。

后端先提供结构化接口：

- `agt harness capabilities`：声明已接入能力与明确未支持的功能。
- `agt harness resource create/update/list/info/delete …`：Harness 资源 CRUD。
- `agt harness models/skills/mcp <资源引用>`：查询该连接的真实能力，模型列表遍历分页。
- `agt harness session current --principal <Agent> --conversation <聊天>`：准备/读取当前快照，不启动线程。
- `agt harness session list/info/models/skills/mcp …`：管理 AGT 保存的会话、查询实例能力。
- `agt harness session configure <记录ID> --model <模型> [--effort <档位>]`：实时校验目录支持，下一轮应用，保留 thread ID。
- `agt harness session rename <记录ID> <标题>`：通过 thread/name/set 同步 Codex；未执行的会话在首次执行后应用。
- `agt harness session compact <记录ID>`：恢复线程并等待压缩 turn/completed；失败保留 dirty 围栏。
- `agt harness session cancel <记录ID>`：请求终止活跃执行/压缩；之后检查并重置上下文。

修改模型、命名、执行和压缩共享数据库互斥围栏。执行中拒绝修改；取消不等于回滚文件操作。
CLI 是本机管理员入口，Web 另行验证当前操作者及目标 Agent 的会话成员身份。
`/review`、`/diff`、任意线程导入/恢复和 fork 本阶段明确不支持；不提供任意 RPC 透传。

Web `/api/harness` 使用固定操作者，验证操作者及目标 Agent 当前成员身份，按当前会话记录 ID
拒绝过期或其他会话的操作；不提供任意 RPC 透传。停止讨论、重置和解散也会取消相关压缩。

本轮无推理的真实 Codex model/list 验证返回 6 个模型，包含 max/ultra 等后端声明的档位；
目录查询不是模型推理端到端验收。协议桩覆盖会话配置隔离、命名、压缩完成及取消围栏。

## 会话与恢复

`runtime/runtime.db` 的 harness_sessions 表保存外部 thread、配置快照、历史游标、已发送历史摘要哈希及 dirty 标记。
作用域包括 Agent 配置、principal、聊天 conversation 和上下文 reset cutoff；资源默认值不纳入会话标识。
无聊天身份的显式 agent run 每次新建独立线程。
恢复时校验已发送历史仍可见且未被修改；不匹配则新建线程。正常恢复只发送增量历史。

### 输入格式（v2）

调度 envelope 仍保存在 AGT Run 中用于审计，但不直接传给 Codex。
显式 agent run 保留输入原文（包括合法 JSON）；普通单条私聊同样仅发送对方原文。
多条消息按 IM seq 排序，显示真实发送者名称、ID 和身份类型。历史背景与新消息
分区，每条消息只出现一次。身份、参与者和通信工具规则置于 developerInstructions。
恢复线程时，通过发布消息的 client_message_id、原 Run、发送者、会话、正文和
外部 thread_id 一起验证，剔除 Codex 已保存的自身答复；不能只按发送者或相同正文过滤。
原生运行器保留结构化 JSON，但当前批次不重复出现在 history，trigger 只引用消息 ID。

输入格式版本纳入会话指纹。升级自 v1 时创建新的干净 Codex 线程，并以可见 Web 历史
作为背景；旧 Codex 线程与 Web 聊天记录保留。旧线程的私有工具上下文不会复制到新线程。

### Web 命令控制通道

共享命令解析器覆盖 Web 与 HTTP 写入边界。单行斜杠命令不写入 IM，也不调用模型：

- `/`、`/help`：可用命令与能力说明。
- `/new`、`/clear`：现有上下文重置确认；后者不删除 Web 聊天记录。
- `/stop`：现有结束讨论确认。
- `/model`：按 Agent 查询 Codex 模型目录与档位，校验后修改当前会话快照，下一轮应用。
- `/status`、`/permissions`：查看当前 Harness 会话与权限快照；内置 Agent 仍使用运行信息页。
- `/rename`：同步 Codex 会话名称；`/compact`：启动并等待引擎压缩，可取消。
- `/skills`、`/mcp`：查询 Codex 自身项目能力，不等于挂载 AGT 的 Skills/MCP。
- `/review`、`/diff`、`/resume`、`/fork` 尚未映射。
  未知命令和带参数命令明确说明不支持，不静默转成用户消息。
- `//model` 通过显式 literal_message 标记发送 `/model` 文本，用于讨论命令本身。
  多行引用、普通句子中的斜杠和 `/tmp/project` 一类路径仍属于普通消息。

命令来源仅限 Web 操作者；Agent 发送的斜杠文本不成为管理员控制命令。
群聊模型/状态按 Agent 分别展示；不支持 @ 定向执行这些命令。
命令语义依据：<https://developers.openai.com/codex/cli/slash-commands/>。
`/new` 通过现有调度围栏终止旧任务，新的 reset cutoff 使下次执行使用新线程。
它不会擦除 Codex 的长期记忆、项目文件或 AGT 聊天历史。

执行前持久化 dirty 防止并发或崩溃后重复执行。失败/中断不自动恢复：检查 Run 和文件，
用 /new 后继续。资源配置改变只影响新会话；已有线程采用快照。进程关闭不会删除 Codex 保存的线程。
外部输入有 1 MiB 传输保护，超过时明确失败；工作上下文压缩交给外部引擎。

## 执行流程

Harness 接管整个任务，外部 Codex 引擎拥有自己的模型与工具循环。AGT 负责身份、
权限、任务责任、会话和运行记录，不通过 Native 的 ModelClient 或模型重试执行它。
普通后续消息仍排队为独立 Run；本次整理不增加运行中的 steering 注入。

### 阶段与状态归属

```text
Runtime: claim → execute_claimed → finish
  Harness: prepare → acquire session lock → prepare input → connect
    Codex: start/resume thread → submit turn → receive protocol events
      event → check fence → persist progress/session facts
      framework tool → prepare → before → execute → after
      native approval → persist request → poll human decision → reply to original RPC
    failure → notify stopping → recover submitted turn → interrupt/confirm
  Harness: collect diagnostics → close connection → settle result/checkpoint
```

| 模块 | 职责 |
| --- | --- |
| `agt-runtime/src/harness.rs` | 宿主编排；连接与关闭外部引擎，串联各阶段 |
| `harness/prepare.rs` | `PreparedHarness` 持有配置、提示词、工具服务、历史和会话库；准备输入与校验续跑指纹 |
| `harness/events.rs` | `HarnessRun` 的取消/租约检查，按事件顺序持久化线程与执行状态 |
| `harness/tools.rs` | `ToolBridge` 的 Framework 工具校验、审计、执行围栏与结果处理 |
| `harness/approval.rs` | 绑定原执行实例的人工审批桥，不运行 Native 审核策略 |
| `harness/finish.rs` | 完成与取消竞态、调用审计结算、会话进度推进和等待 checkpoint |
| `harness_input.rs` | IM 历史到外部输入的转换与去重 |
| `harness_sessions.rs` / `harness_sessions/lifecycle.rs` | 会话配置与所有权租约、执行状态、恢复责任 |
| `agt-harness/src/codex.rs` | 稳定的 `Codex` 接口和事件类型 |
| `codex/transport.rs` | 子进程、限长协议帧、RPC、超时、stderr 诊断和进程组回收 |
| `codex/turn.rs` | 线程准备、正常回合事件循环和动态工具 RPC 响应 |
| `codex/approval.rs` | 原生审批选项、请求关联及响应编码 |
| `codex/cancellation.rs` | 已提交回合的停止恢复、中断请求及终态确认 |
| `codex/control.rs` | 元数据操作与显式压缩 |

`PreparedHarness` 拥有准备资源；`HarnessRun` 借用它并持有会话执行锁。
配置快照必须在取得锁之后读取。事件回调修改 Run 记录，工具桥独立持有等待标志和
失败计数，结束时再交给收尾阶段。没有插件注册表，也不共享 Native transcript。

### 工具阶段

- `prepare_tool_call`：检查取消、等待状态、服务可用性、工具前缀和当前聊天发送限制。
- `before_tool_call`：创建脱敏审计并保存 Started。
- `execute_tool_call`：调用固定身份的 ToolService；保留普通工具的 scheduler 围栏，以及
  task/harness 控制工具已有的专用保护路径。
- `after_tool_call`：记录等待状态、成功/失败审计与失败计数，将结果交回协议适配器。

Codex 协议层先检查动态工具是否在暴露列表中，再进入宿主工具桥。工具业务失败以
失败工具结果回复引擎，不重新提交回合。这些阶段只管理 AGT Framework 工具；Codex
内部的命令、文件、MCP 等工具由 Codex 自己驱动，AGT 接收其进度事件。

### 不可改变的时序契约

1. 线程 ID 在收到 Thread 事件后持久化；默认模型与推理配置在对应事件后保存。
2. Submitted、Turn、Settled 分别推进执行状态。失败/中断的已确认输入进度必须在
   释放执行围栏前保存，不能统一挪到成功收尾中。
3. 中断 RPC 的响应不等于中断完成。只接受匹配线程与回合的终态通知。
4. 取消发生在读取协议帧之后时保留 pending 帧；尚无 turn ID 时进行有界恢复。
   停止恢复只读协议事实，不执行工具、不重放已提交任务。
5. Stopping 是诊断通知，其失败不能跳过中断清理；继续保留原始停止错误。
   InterruptRequested 与 Settled 的控制回调仍保留原有检查。
6. 已确认完成可以赢得与取消的竞态；连接丢失不视为已确认失败，也不自动重放。
7. 等待续跑保存原有 session/thread/config 指纹 checkpoint；恢复仍检查三者一致。
8. 外部进程在收尾前关闭；连接释放时回收进程组，包括外部工具创建的子进程。

### 验证

协议 fixture 是本地 Python app-server，不连接真实模型。
`agt-harness` 测试覆盖成功与工具失败、工具白名单、取消前后 start ack、错回合终态、
完成/取消竞态，以及执行和压缩的停止通知失败后仍中断确认。
Runtime 原有测试覆盖权限、会话锁、终态分类、输入历史、工具控制和等待恢复。

原生人工审批、取消与回传状态见[权限审批](permissions.md)。

## 执行状态与租约

本模块将执行结果和并发执行权分开。`harness_sessions` 保存配置、引擎线程 ID 和已接收输入进度；
`harness_operations` 保存每个会话最近一次操作的状态和租约。Run 继续承担模型执行审计，任务队列继续管理依赖和汇报。
操作的私有输出不通过会话查询、SSE 或 Web 状态接口返回。

### 状态

| 状态 | 含义 | 能否开始下一操作 |
|---|---|---|
| preparing | 连接、准备和提交过程；submitted 标记是否进入可能已发送的边界 | 否 |
| running | 引擎已接受 | 否 |
| canceling | 请求取消，等待确认 | 否 |
| reconciling | 执行者租约过期，检查持久状态 | 否 |
| succeeded | 结果已保存 | 是，原持有者已释放时 |
| failed | 提交前失败、明确拒绝或引擎确认失败 | 是，原持有者已释放时 |
| canceled | 提交前取消，或请求停止后引擎确认中断 | 是，原持有者已释放时 |
| interrupted | 引擎主动确认中断，未收到本地停止请求 | 是，原持有者已释放时 |
| needs_review | 已提交但无法确认结果，或未能可靠保存结果 | 否；先检查原线程及副作用 |

失败和取消不回滚项目文件，不自动重做旧任务。界面 busy 是状态与持有者的派生值，不是另一套持久状态。
配置和重命名也使用同一执行权，避免与模型执行并发修改会话快照。

### 获取、续租和隔离

原子获取执行权，分配 UUID 执行归属和递增 generation。租期 30 秒，独立线程每 5 秒续租，
因此长工具调用不会仅因为没有流式消息而失去执行权。续租不得复活过期租约。
提交请求之前先保存可能已提交的标记，避免发送成功但标记尚未写入的窗口。
会话写入和释放均核对 owner、generation 和 expires。过期持有者的写入、释放均被拒绝。
运行进程仍须尊重 scheduler 的授权；租约不提供新的资源或工具权限。

### 正常完成、失败与取消

正常完成先保存会话输入进度和结果，再释放执行权。失败或中断得到确认后，先记录引擎已收到的输入，
再结束操作，避免新请求再次注入同一批历史。明确拒绝不推进输入进度。

取消不是终态：发送 turn/interrupt 后，最多等待 2 秒匹配原 thread/turn 的终态通知。
RPC 应答本身不算中断确认；无关 turn 的通知不能解除执行保护。确认中断后释放执行权，允许继续会话。
若取消前已经完成且成功取得结果，保留完成结果；讨论停止后的公开发布仍受 scheduler 授权限制。
压缩同样区分请求接受、终态和取消；只有得到可识别的 turn 才能发送定向中断。

### 进程强杀与恢复边界

SIGKILL 不运行析构函数。过期租约由 scheduler 的修复扫描、会话查询或下次获取执行权时回收：

- 未进入提交边界：结束为 failed，可以继续。
- 已持久保存明确失败/取消：保留终态，移除失效持有者。
- 可能已提交且无可靠终态：经过 reconciling 转为 needs_review，不自动发送新请求。

当前 Codex 适配器没有实现跨进程重新附着活跃 turn，因此不能把失联解释为远端已停止，
也不能从不存在的连接中自动取回完成结果。needs_review 时保留原线程 ID，用户应检查 Codex 线程和项目产出，
确认旧执行停止后通过现有 /new 开始新上下文。旧执行仍可能留下外部副作用。
这释放的是无主的执行租约，不是绕过未确认结果的业务保护。

旧 dirty=1 数据升级为 needs_review，旧配置、线程 ID 和进度保留，不自动清理或重放。
状态变化进入已有权限隔离事件源；续租不会产生进度事件或被解释成模型工作进展。

### 验证

`crates/agt-runtime/src/scheduler/tests/harness_session_lock_tests.rs` 集中覆盖启动失败、明确拒绝、失败后继续、
取消确认、压缩取消、真实子进程强杀、租约到期、旧归属隔离和旧数据迁移。
协议桩位于 `crates/agt-harness/tests/fixtures/codex.py`，不调用真实模型。

### 任务终止诊断

讨论期限只限制新的 admission（包括子任务、恢复片段和通知模型调用）。在途执行继续；
自然到期不能撤销其运行权限、调用计数或结果投递。停止、上下文重置、权限变化仍撤销执行权限。
对账单独检查执行归属，允许仍持有有效租约的执行者完成停止确认。

Run 与 Work 使用 succeeded/failed/canceled/interrupted/outcome_unknown 对应的明确终态
（Run 成功名为 completed）。超时且确认中断为 failed；主动取消且确认停止为 canceled；
引擎自行中断为 interrupted；提交后断线、失去归属或缺失可保存结果为 outcome_unknown。
取消请求期间 execution_status=canceling；该值由取消意图与运行事实派生，不新增独立任务状态机。

termination 保存 code/source/observed_at/interrupt_requested_at/confirmed_at/confirmation/side_effects/actor。
首个原因保留，不用清理错误覆盖；副作用可能存在与停止是否确认分开表达。task_get 提供安全诊断
和 run/thread/turn/session 标识，不携带进程 stderr。主机 run info 的 execution.diagnostics
可查看子进程退出码、信号和最多 8 KiB 的 stderr 尾部；常见凭据行被过滤，仍按主机私有日志处理。
已有已完成结果不会因迟到取消丢失，但向失效上下文的发布仍受权限检查。

验证位于 `crates/agt-runtime/src/scheduler/tests/work_tests/expiry.rs`、`work_tests/termination.rs` 和 `harness_session_lock_tests.rs`。

## 后续接入契约

Claude Code、其他 Harness 以独立适配器实现相同的任务/事件/工具桥接边界，
必须核实结构化输出、恢复、取消与工具权限能力；不按终端文字猜测状态。
目前只注册 Codex，不提供假可用选项。后续扩展远程连接、临时执行工具前，
分别补充权限和生命周期测试。远程任务取消不能等同于断开本地连接。

## 测试范围

无真实模型调用的协议桩覆盖启动/恢复、动态工具、流式与最终消息分离、空输出、错误、
取消、未知交互与进程退出；调度测试覆盖提及、私聊、增量历史、/new 及旧任务禁止发布。
资源测试覆盖旧 Agent 配置、互斥引用、Harness 依赖删除、Manager 写权限。
Web 验证创建/编辑Harness 与 Agent、缺少模型时选择外部 Harness、桌面和移动端布局。
真实账户端到端执行需另行验收，不能以连接检查代替。
