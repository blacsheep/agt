# 上下文管理

实现范围：运行内自动压缩、跨运行会话摘要、显式重置。长期记忆与跨会话知识库不在本阶段。
参考 OpenAI Codex `ce254df05a3162a93d8f3357ff4dd86582c534b7` 的工作上下文 / 持久化历史分离设计；
采用普通模型摘要，兼容现有 Chat Completions 和 Responses，不要求专用压缩接口。

## 模块边界

| 模块 | 职责 |
| --- | --- |
| agt-model | `capabilities.context_window` 配置、校验、资源读写摘要 |
| inference/context.rs | token 估算、输入预算、工具输出裁剪、无工具摘要调用 |
| runtime/engine/native/request.rs | 在完整工具批次结束后压缩、保留固定指令与当前请求、Run 压缩诊断 |
| runtime/conversation_context.rs | 为一个 principal / conversation 组装可见历史、更新会话摘要 |
| runtime/context_store.rs | `runtime/runtime.db` 中 conversation_summaries 表的摘要持久化 |
| runtime/scheduler.rs | 摘要调用的调度授权、重置权限、与旧任务发布之间的围栏；schedule_store 在 runtime.db 保存重置起点 |
| agt-im | 原始消息、分页与成员可见区间；不依赖模型或 Runtime |
| CLI / Web | 同一 Runtime 重置接口的适配和交互；Node 不生成或管理摘要 |

Agent 私有工具执行摘要仅存于 `runtime.db` 的 Run `context` 字段；不写入 IM，
也不注入其他 Agent 的上下文。会话摘要只提取该 principal 可见的公共对话消息，
按 `(conversation, principal)` 保存，与 Agent 配置指纹和来源内容摘要绑定。
这不是所有 Agent 共用的全局记忆。

## 预算与压缩

Model 默认窗口为 272000 tokens，旧配置缺省兼容；有效范围 4096–2000000。
不同 Provider 的窗口无法从模型名称可靠推断，应按模型文档填写。
CLI `agt model update <model> --context-window 128000`，Web 新建模型也可设置；
资源工具 create 用 `params.context_window`，update 直接使用 `context_window`。
这个参数不发送到模型 API，也不设置 `max_tokens` / 输出上限。

本地估算与 Codex `approx_token_count` 对齐：UTF-8 字节数除以 4，向上取整；
不再对非 ASCII / 中文额外加算。这只是粗略回退，不是精确 tokenizer。
运行时优先使用上次模型响应的 usage 总量（输入 + 输出，包括已计入的缓存输入），
再加该响应后新增工具结果的本地估算；无 usage 时回退到完整请求估算。
Chat Completions 读取 total_tokens 或 prompt_tokens + completion_tokens；Responses
读取 total_tokens 或 input_tokens + output_tokens。压缩后清除旧 usage 基准并重新估算，
不把旧窗口的 token 数带到新窗口。计数元数据不写入模型输入。
自动压缩阈值为模型窗口的 90%，有效输入窗口为 95%，与 Codex 默认比例一致；
默认窗口 272000 对应 244800 开始压缩、258400 有效输入预算。没有 180000 的额外预算上限。
不再将 token 预算乘以 4 当作传输字节上限。服务端 usage 尚未到阈值时，不因内部历史超过 1 MiB 而失败。
无 usage 的 Responses 回退估算只计原始输出项，不重复计入用于工具执行的规范化副本。
未知 tokenizer 不能保证完全避免上游 context-length 错误。
HTTP JSON 和两种流协议明确返回 context_length_exceeded 时，记录 provider_context_exceeded，
不走同一请求的普通网络重试。运行内有可压缩历史时强制摘要，然后重新采样一次；仍超限则失败，
成功响应后才重置这次恢复额度。固定输入本身超限时不假装可以靠删除执行历史解决。
HTTP 413 或未经结构化分类的错误不等同于 token 超窗，不自动触发压缩。
摘要请求自身被明确判定超窗时，将当前分块减半后重试；保留未处理文本，直到全部处理完。
分块无法继续合理缩减时明确失败，不删除原始聊天或重放已完成工具。
固定指令、当前请求和工具 schema 本身过大时明确失败，不静默丢弃当前任务。

单条工具结果保留预算内的首尾文本，标注截断并建议缩小查询；不修改 IM 消息。
完整工具批次结束后，超过输入预算则对较早执行数据分块摘要，保留系统指令和原始请求。
摘要不带工具 schema，不会执行或重放工具；丢弃已摘要的整组调用与结果，
不会把孤立 tool result 发给 Provider。Responses 的原始 reasoning/加密项与 Provider
私有 reasoning 字段不进入摘要输入。已成功发送的消息去重状态留在 Runtime 内存中。

摘要目标长度只用于提示，不再把 1638 / 2048 作为摘要成功的硬门槛。
与 Codex 普通摘要路径一致，摘要完成后重建上下文、重新计数；本框架保留整体输入预算校验。
运行内检查固定指令、当前请求、摘要和工具 schema 的整体大小；会话摘要与近期历史
共享输入预算的一半，另一半留给触发消息、指令和工具。分块摘要请求自身也做输入预算检查。
真正装不下时返回 context_overflow，含 stage、estimated_tokens 和 available_tokens，
不安装失败的压缩结果。空摘要走现有最多 10 次请求重试，含工具调用的摘要仍拒绝。
未成功压缩前不覆盖 checkpoint；失败后保留旧 checkpoint、原始 Run 请求与 IM 历史。
Run `context` 保存最近摘要、压缩次数和输入计数（estimated_tokens 为兼容字段名）；
`run calls` 单独记录 compaction、累计请求次数、压缩前后计数、预算和 count_source。
trigger 区分 token_budget（正常阈值）和 provider_context_rejected（服务端拒绝后的补救）。
count_source 区分服务端 usage + 本地增量与纯本地估算；request_attempts 包括分块调用和网络重试。
model_steps 继续统计普通逻辑采样（含子 Run），压缩调用单列；调度器 calls 包含实际摘要请求。
不持久化完整私有工具 transcript，不支持重启后自动恢复中断的工具循环。

会话历史预算为输入预算的四分之一。超过时将旧消息和已有摘要一起分块整理，
保留预算内最多 20 条近期原文，并留一半历史预算供后续消息增长，避免逐条重新摘要。
预算内的历史不会仅因超过 20 条而丢失。摘要由同一个配置模型生成，也会产生模型费用。
后续回复 / 参与判断加载已保存摘要及尚未摘要的消息；重启后亦然。
读取始终通过 IM 成员可见区间，来源消息删除、可见区间改变或配置指纹变化时重建摘要。
当前为保证来源校验，会分页扫描重置点后的可见历史；本机扫描量随历史增长，
后续可通过 IM 变更版本优化，但不能绕过成员可见性和删除校验。

## 重置语义与权限

Web：单独输入 `/new` 并发送（也支持 Enter），确认后重置；命令不写入聊天记录或发送给模型。
也可使用会话信息 / 群信息 → 模型上下文 → 重置上下文。两种入口使用同一 Runtime 接口。
CLI：`agt schedule reset <conversation> --actor <human-principal>`。
私聊允许仍在会话中的人类参与者；群聊允许仍在群内的人类群主 / 管理员。
观察 Agent 会话的页面不提供重置，Agent 资源管理权限也不会隐式授予此操作。

1. 在 scheduler 写围栏内读取消息序号，持久化新的上下文起点并删除该会话所有 Agent 摘要。
2. 结束源于或涉及该会话的讨论，包括同一讨论在其他群的工作。
3. 后续模型请求、工具调用、摘要写入与自动发布复查围栏。已发出的外部请求不能撤销。
4. 新消息才能开始新讨论；重复授权旧消息、点击继续、服务重启不会恢复旧上下文。

重置序号、摘要清除和讨论结束状态在 runtime.db 同一个事务提交，避免跨会话任务在进程崩溃后继续。
摘要缓存先清除；若事务未提交即中断，最多重建缓存，不会丢失 IM 历史。
Scheduler schema 从 v3 加法升级到 v4，保留现有讨论与任务；更早的不兼容版本仍拒绝。
重置保留聊天记录、历史 Run 与资源配置。当前请求和工具 transcript 随旧 Run 停止；
不会物理擦除历史诊断。消息查询工具仍可主动读取重置前的可见消息，因此重置不等于遗忘或删除。
未来长期记忆需要另行提供生成、使用、删除及共享范围管理，不能复用“重置上下文”冒充全部遗忘。

## Codex 源码依据

- [UTF-8 字节估算](https://github.com/openai/codex/blob/ce254df05a3162a93d8f3357ff4dd86582c534b7/codex-rs/utils/string/src/truncate.rs#L71)
- [服务端 usage + 本地新增项](https://github.com/openai/codex/blob/ce254df05a3162a93d8f3357ff4dd86582c534b7/codex-rs/core/src/context_manager/history.rs#L677)
- [摘要重建与重新计数](https://github.com/openai/codex/blob/ce254df05a3162a93d8f3357ff4dd86582c534b7/codex-rs/core/src/compact.rs#L356)

默认窗口取 Codex 未知模型回退值 272000，压缩阈值和有效窗口比例对齐 90% / 95%。
已有配置明确保存的 context_window（包括之前写入的 32768）保留，缺省配置和新建模型使用新默认值。
要调整已保存的模型，请执行 `agt model update <model> --context-window 272000`。
分块调用、多 Agent 会话分区仍属于 AGT 的适配，不宣称所有上下文策略与 Codex 完全相同。

本次超限恢复参考 Codex `bba48e5ce8bbcaee4af0bb2c71ad2936e4e24ea9`：
[请求发送无通用 1 MiB 限制](https://github.com/openai/codex/blob/bba48e5ce8bbcaee4af0bb2c71ad2936e4e24ea9/codex-rs/codex-api/src/endpoint/responses.rs#L70-L95)、
[本地压缩超窗补救](https://github.com/openai/codex/blob/bba48e5ce8bbcaee4af0bb2c71ad2936e4e24ea9/codex-rs/core/src/compact.rs#L302-L345)。
AGT 使用缩小分块而非删除最旧数据，保证待摘要的原始内容不会在补救时被跳过。
