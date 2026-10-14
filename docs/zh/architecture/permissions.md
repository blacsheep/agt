# 权限审批与交互观察

Native 工具权限与 Harness 原生审批采用独立路径，共用审批账本和人工界面。

## Native 工具权限

`engine/native/tools.rs` 在工具参数准备后、工具开始执行前进入 `permission.rs`。权限循环固定工具参数、选择策略、轮询结果并检查取消，决定持久化后再次检查执行围栏。 `PermissionPolicy` 的 `poll/cancel` 接口由 Manual、AutoAllow、ModelReview 实现。策略只产生决定，不能执行工具，也不能绕过 Framework ACL、MCP 挂载或调度授权。拒绝会生成对应 call ID 的工具结果并继续模型循环；取消结束运行。

工作区 `config.yaml`：

```yaml
permissions:
  mode: model_review
  harness_mode: manual
  reviewer_model: default/reviewer
  review_timeout_seconds: 30
  request_timeout_seconds: 600
```

默认 `model_review`，默认审核模型未配置，因此转人工。审核模型引用已有 Model 资源，独立于执行 Agent；使用固定审核指令，无工具，一次模型请求，无自动重试。模型的明确拒绝不会转成批准。缺配置、错误、超时、非法输出、不确定结论转人工。 `manual` 直接等待人工；`auto_allow` 必须显式配置，仅作用于 Native。Web「权限审批」面板与「运行设置」可编辑这些字段。每个 Native Run 持有一份策略快照，每次调用前只比对 `config.yaml` 的文件标识，文件变化后才重新读取；保存配置会发出唤醒通知。因此新调用总是使用最新策略，等待期间配置变化会取消旧审核并重新选择策略。`model_review` 下每次调用重新加载审核 Model，资源修改对下一次请求生效。策略变化时同步更新 Run 的执行指纹；当前策略及审核模型指纹更新到审批记录，凭据不写入审批记录。

等待审批时不再定时轮询数据库：人工决定、审核模型返回和配置保存都会唤醒等待中的请求；无通知时每秒兜底检查一次租约、过期与审核超时，取消信号在内存中每 100ms 检查。已作出的决定不撤回，审批总截止时间不因切换模式延长。

人工可在模型审核完成前决定，版本条件更新确保迟到结果不能覆盖人工决定。取消会立即停止权限等待；底层阻塞 HTTP 最迟由审核请求超时结束，不保证立即断开 socket。当前模型审核不单独统计 usage/费用，也不提供动态加载的策略插件。

## Harness 原生审批

Codex 宿主使用 `on-request` + `user`，不经过 AGT 模型审核。独立的 `permissions.harness_mode` 默认为 `manual`；显式选择 `auto_allow` 后，宿主对当前等待与后续请求自动回传引擎提供的 `accept` 或所请求的本回合权限。模式不改变 sandbox、不授予持久 session 权限。Codex 回合在协议帧间隙轮询待审批请求；宿主只在收到唤醒通知或每秒兜底时读取审批存储和调度围栏，其余轮询直接返回“尚无决定”。 `agt-harness` 解析原生请求，`harness/approval.rs` 绑定宿主执行；人工或自动决定回传同一连接、同一回合的原 JSON-RPC ID。协议循环等待时仍处理进度、终态、取消，并维持续租。

支持的 app-server 请求：

- commandExecution：原请求可用选项中的 accept、acceptForSession、decline、cancel。
- fileChange：同上四种原生决定；附带引擎已发出的同回合 item 内容，缺失 diff 时明确提示。
- permissions：授予原请求 profile（仅本回合）、空授权（拒绝）、取消回合。

命令拒绝由引擎继续处理；permissions 的取消通过 turn/interrupt 处理。未知请求、持久规则修改等不支持的选项不自动批准。原生 session 授权由 Codex 管理。模式保存在工作空间配置，关闭页面后仍生效；当前仅实现 Codex 适配器。协议依据 Codex 0.156.1 schema；自动测试使用本地 app-server 桩，未进行真实账户推理验收。

## 人工入口与身份

「交互观察 → 权限审批」可以查看队列、请求参数、决定来源、截止时间和回传状态。面板顶部提供持续生效的处理模式，不再使用一次性批量批准按钮。逐条人工决定仍要求主动选择，不预选批准。切换观察身份后旧详情清空；观察身份只读。Web 写入使用控制台启动身份，Rust 进一步要求它是工作区拥有者且仍为对应会话成员。普通人类成员与 Agent 不能批准。无会话的本机 Run 通过本机 CLI 管理。

```sh
agt --home ~/.agt --output json permission list
agt --home ~/.agt --output json permission info REQUEST_ID
agt --home ~/.agt --output json permission decide REQUEST_ID \
  --expected-revision 2 --decision '"allow"' --reason '已确认本次操作'
```

`--decision` 必须是详情 `request.options[].value` 中的原样 JSON：Native 用 `allow/deny/cancel`，Harness 使用其原生选项。CLI 不带 `--principal` 表示本机管理员；带身份则强制执行可见性和拥有者校验。列表每页最多 100 条，使用 `--before` 加上一页最后一个审批 ID 查询下一页。

审批存储在 `runtime/runtime.db` 的 approvals 表，变化进入 Events outbox，不在 SSE 广播请求正文。人工决定使用 revision 条件更新；审批绑定 Run、执行实例和原请求。审批过期、取消或宿主丢失后不能继续批准。宿主异常退出的记录通过 30 秒租约失效；新执行实例不能复用旧决定，不跨进程重新发送审批响应。 `sent` 仅表示写出了响应，`resolved` 表示审批阶段已结束，都不等于工具或任务成功。写响应过程崩溃可能标记为 `unknown`，必须人工核对，不能保证外部副作用 exactly-once。Harness 审批上限为 600 秒，仍受原 Run 总时限约束。

## 交互观察

任务边来自权限过滤后的 `task_list`，使用 requester → assignee，不依赖聊天消息。同一对参与者的任务聚合成边，按任务 ID 去重；独立分页，点击读取 `task_get`。节点显示当前绑定引擎，任务详情显示 Run 实际执行引擎，避免改绑后混淆。任务/审批查询失败各自显示错误，不阻断消息查询。图内审批摘要取最新 100 条可见记录，完整历史通过审批面板分页读取。没有 work_id 的会话执行也能在审批队列与节点入口处理。

验证覆盖原生批准/拒绝/取消、RPC 身份与重复请求、等待期间终态、审批 CAS/过期/执行实例隔离、拥有者和成员校验、Native 拒绝不执行及错误转人工，以及浏览器的无消息委派图、人工提交与身份切换。
