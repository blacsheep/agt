# 外部执行引擎与运行环境

## 决策

Harness 接入是框架能力；运行环境是可复用配置资源；Agent 是独立身份与角色。
一个 Agent 选择内置 Model 或外部 environment，二者互斥。旧配置继续使用 Model。
环境不是群成员，外部框架的内部子 Agent 不会自动成为 AGT 联系人。

## 模块与边界

- `agt-harness`：运行环境配置和安全文件存储、Codex app-server 协议适配、进程生命周期。
- `agt-agent`：model/environment 互斥引用、Prompt/Skill/内置工具挂载。
- `agt-service`：环境 CRUD、删除引用检查、统一资源工具契约。
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

环境配置：engine=codex、服务器绝对 executable、working_directory、可选 model、
sandbox=read_only/workspace_write、timeout_seconds（默认 1800）。不保存 API Key，
沿用运行账户的 Codex 认证与配置。工作区文件可能共享，协作开发应明确分配目录/worktree。
权限策略固定 never：执行选定 sandbox 内允许的操作，不批准权限升级。
目前遇到交互授权/用户输入请求会停止并说明原因；没有自动同意或伪造用户输入。
环境写入仅供 Manager，资源管理 Agent 可以读取并引用管理员配置的环境。
AGT 工具权限不构成外部终端的文件隔离；实际文件边界由 Codex sandbox 与操作系统控制。
本轮在 Linux 验证进程组清理及 SIGINT/SIGTERM 取消；Windows 的完整进程树终止尚未验收。

Prompt 和 Skill 正文作为 developerInstructions 注入；allowed-tools 约束未实现时拒绝运行。
框架工具通过 Codex dynamicTools 桥接，逐次使用 AGT 固定身份校验并检查取消围栏。
message_send 可联系其他会话；向当前会话发送时拒绝，由调度器唯一发布最终答复，避免重复消息。
AGT 挂载 MCP 和显式 --delegate allowlist 首版不支持，配置/执行时明确拒绝。
Codex 自身配置中的工具仍由 Codex 管理，其权限和认证不由 AGT 冒充控制。

## 会话与恢复

`runtime/harnesses.db` 保存外部 thread、历史游标、已发送历史摘要哈希及 dirty 标记。
作用域包括 Agent principal、聊天 conversation、上下文 reset cutoff、实际配置指纹。
无聊天身份的显式 agent run 每次新建独立线程。
恢复时校验已发送历史仍可见且未被修改；不匹配则新建线程。正常恢复只发送增量历史。
`/new` 通过现有调度围栏终止旧任务，新的 reset cutoff 使下次执行使用新线程。
它不会擦除 Codex 的长期记忆、项目文件或 AGT 聊天历史。

执行前持久化 dirty 防止并发或崩溃后重复执行。失败/中断不自动恢复：检查 Run 和文件，
用 /new 后继续。环境配置改变也会建立新的线程。进程关闭不会删除 Codex 保存的线程。
外部输入有 1 MiB 传输保护，超过时明确失败；工作上下文压缩交给外部引擎。

## 后续接入契约

Claude Code、其他 Harness 以独立适配器实现相同的任务/事件/工具桥接边界，
必须核实结构化输出、恢复、取消与工具权限能力；不按终端文字猜测状态。
目前只注册 Codex，不提供假可用选项。后续扩展交互审批、远程连接、临时执行工具前，
分别补充权限和生命周期测试。远程任务取消不能等同于断开本地连接。

## 验收要求

无真实模型调用的协议桩覆盖启动/恢复、动态工具、流式与最终消息分离、空输出、错误、
取消、未知交互与进程退出；调度测试覆盖提及、私聊、增量历史、/new 及旧任务禁止发布。
资源测试覆盖旧 Agent 配置、互斥引用、环境依赖删除、Manager 写权限。
Web 验证创建/编辑环境与 Agent、缺少模型时选择外部环境、桌面和移动端布局。
真实账户端到端执行需另行验收，不能以连接检查代替。

## 本轮验收（2026-09-15）

- 工作区 Rust 测试 293 项通过；后续工具消息边界／连接失败审计修订通过外部执行定向回归。
- Clippy 全工作区所有 target，禁止警告；格式检查与 diff 空白检查通过。
- Web 接口测试 18 项通过；Harness 创建编辑、工作区、会话管理、成员状态浏览器检查通过。
- Linux SIGTERM 测试确认 CLI 经取消路径结束 Run 并回收本地 Codex 进程。
- 本机 codex-cli 0.154.0 的真实 app-server 握手及 account/read 成功；未发起真实模型推理。
- 真实模型执行、Windows 进程树清理、其他 Codex 版本以及其他引擎仍需各自验收。
