# Harness 资源与外部执行引擎

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
权限策略固定 never：执行选定 sandbox 内允许的操作，不批准权限升级。
目前遇到交互授权/用户输入请求会停止并说明原因；没有自动同意或伪造用户输入。
Harness 写入仅供 Manager，资源管理 Agent 可以读取并引用管理员配置的 Harness。
AGT 工具权限不构成外部终端的文件隔离；实际文件边界由 Codex sandbox 与操作系统控制。
本轮在 Linux 验证进程组清理及 SIGINT/SIGTERM 取消；Windows 的完整进程树终止尚未验收。

Prompt 和 Skill 正文作为 developerInstructions 注入；allowed-tools 约束未实现时拒绝运行。
框架工具通过 Codex dynamicTools 桥接，逐次使用 AGT 固定身份校验并检查取消围栏。
message_send 可联系其他会话；向当前会话发送时拒绝，由调度器唯一发布最终答复，避免重复消息。
AGT 挂载 MCP 和显式 --delegate allowlist 首版不支持，配置/执行时明确拒绝。
Codex 自身配置中的工具仍由 Codex 管理，其权限和认证不由 AGT 冒充控制。

## 会话与恢复

`runtime/harnesses.db` 保存外部 thread、配置快照、历史游标、已发送历史摘要哈希及 dirty 标记。
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
- `/review`、`/diff`、`/resume`、`/fork` 和交互审批尚未映射。
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

## 后续接入契约

Claude Code、其他 Harness 以独立适配器实现相同的任务/事件/工具桥接边界，
必须核实结构化输出、恢复、取消与工具权限能力；不按终端文字猜测状态。
目前只注册 Codex，不提供假可用选项。后续扩展交互审批、远程连接、临时执行工具前，
分别补充权限和生命周期测试。远程任务取消不能等同于断开本地连接。

## 验收要求

无真实模型调用的协议桩覆盖启动/恢复、动态工具、流式与最终消息分离、空输出、错误、
取消、未知交互与进程退出；调度测试覆盖提及、私聊、增量历史、/new 及旧任务禁止发布。
资源测试覆盖旧 Agent 配置、互斥引用、Harness 依赖删除、Manager 写权限。
Web 验证创建/编辑Harness 与 Agent、缺少模型时选择外部 Harness、桌面和移动端布局。
真实账户端到端执行需另行验收，不能以连接检查代替。

## 本轮验收（2026-09-15）

- 工作区 Rust 测试 293 项通过；后续工具消息边界／连接失败审计修订通过外部执行定向回归。
- Clippy 全工作区所有 target，禁止警告；格式检查与 diff 空白检查通过。
- Web 接口测试 18 项通过；Harness 创建编辑、工作区、会话管理、成员状态浏览器检查通过。
- Linux SIGTERM 测试确认 CLI 经取消路径结束 Run 并回收本地 Codex 进程。
- 本机 codex-cli 0.154.0 的真实 app-server 握手及 account/read 成功；未发起真实模型推理。
- 真实模型执行、Windows 进程树清理、其他 Codex 版本以及其他引擎仍需各自验收。

### 输入与命令修订验收

- 全工作区 Rust 测试、Clippy（所有 target，禁止警告）、格式及 diff 检查通过。
- Web 接口测试 20 项通过；Harness 与工作区浏览器回归通过，覆盖命令拦截、显式普通文本发送、模型状态与上下文重置。
- 协议桩验证私聊原文、群聊发送者与顺序、已发布回复去重；本次未调用真实模型。

## Harness 资源与执行会话（v3）

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
`/review`、`/diff`、任意线程导入/恢复、fork 和交互审批本阶段明确不支持；不提供任意 RPC 透传。

Web `/api/harness` 使用固定操作者，验证操作者及目标 Agent 当前成员身份，按当前会话记录 ID
拒绝过期或其他会话的操作；不提供任意 RPC 透传。停止讨论、重置和解散也会取消相关压缩。

本轮无推理的真实 Codex model/list 验证返回 6 个模型，包含 max/ultra 等后端声明的档位；
目录查询不是模型推理端到端验收。协议桩覆盖会话配置隔离、命名、压缩完成及取消围栏。

### v3 验收

- 全工作区 Rust 测试通过；后续默认模型固定与线程内切换修订通过 44 项运行时单元测试。
- Codex 协议桩 4 项测试通过；全工作区 Clippy 所有 target 禁止警告、格式和 diff 检查通过。
- Web 接口 21 项测试通过；Harness 与工作区浏览器回归通过，手机/桌面模型选择布局已核验。
- 验证真实 Codex 模型发现，不发起推理；真实推理、其他引擎和交互审批仍未验收。
