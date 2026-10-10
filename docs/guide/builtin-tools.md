# Agent 内置工具

内置工具按组挂载到 Agent 定义的 `builtin_tools`，默认全部关闭，由操作员显式开启。
Web 在创建/编辑 Agent 时勾选，CLI 用 `--builtin-tool`、`--add-builtin-tool`、`--remove-builtin-tool`。
模型侧的工具名带 `framework_` 前缀。

| 工具组 | 用途 | 说明 |
| --- | --- | --- |
| `resource_management` | 管理资源、Agent 身份与绑定、Harness 会话 | 本文 |
| `task_collaboration` | 委派、等待、汇报显式工作任务 | [异步任务协作](tasks.md) |
| `local_execution` | 在宿主机执行命令（还需 `execution.enabled`） | [运行配置与记忆](config-and-memory.md#本机执行) |

消息收发、会话管理和 Memory 工具对所有托管 Agent 固定开放，不需要挂载。

以下各节说明 `resource_management`。

## Agent 授权范围

Agent 定义增加 `builtin_tools: [resource_management]`，缺省为空。
Web 创建／编辑 Agent 提供“资源管理”复选框，操作员勾选并保存即授予权限。
CLI 提供 `--builtin-tool`、`--add-builtin-tool`、`--remove-builtin-tool`。
未知工具组、配置文件及创建请求中的重复挂载拒绝保存；CLI 添加已挂载的组采用幂等行为。

授权作用于当前框架 home 中的全部七类资源，不局限于该 Agent 自身的挂载资源。
可以读取、创建、修改、删除 Provider、Model、Prompt、Skill、MCP、Harness 和 Agent 定义；
因此也能修改其他 Agent 的后续配置，包括其内置资源工具挂载。不会影响已加载的运行快照。
不把执行身份升级为 Manager。允许 principal_create（仅 Agent 类型）、agent_bind、
agent_set_enabled、agent_unbind、binding_list；普通用户等身份创建仍仅 Manager 可用。
不绕过通信工具的会话成员权限。

删除继续使用现有“计划 → 精确确认”协议；资源工具允许请求递归删除依赖者。
默认 Prompt、仍托管的 Agent、损坏配置和悬空引用继续受到现有删除保护。
分类删除仅允许空的非 default 分类。模型可以自行完成两次工具调用，
该确认协议不是 Web 人工审批弹窗；挂载资源管理意味着授权这些写入和删除能力。

## 统一工具契约

| 工具 | 参数 | 行为 |
| --- | --- | --- |
| resource_list | kind | 七类资源摘要，敏感字段脱敏 |
| resource_read | kind, reference | 读取定义或正文，Provider 不返回密钥 |
| resource_write | kind, reference, create, data | 创建或修改，复用现有类型化校验 |
| resource_delete | kind, reference, recursive?, confirm? | 先获取计划，再提交精确有序确认 |
| provider_models | reference | 使用保存的密钥请求 Provider 的 /models，不执行推理 |
| resource_test | kind: provider/mcp, reference | 仅配置校验，不启动 MCP、不测网络连通 |
| resource_category_list | kind | 列出分类 |
| resource_category_create | kind, category | 创建分类 |
| resource_category_delete | kind, category | 删除空的非默认分类 |

MCP 和 CLI `tool call` 继续由 `--manager` 开放这组工具；
内置挂载通过单独资源权限开放同一组，运行时名称添加 `framework_` 前缀。

Provider 创建支持 `{base_url, api_key}` 直接输入；保留 api_key_env 兼容旧 MCP 客户端，
二者互斥，创建必填一种非空密钥，更新可省略密钥。所有返回与错误继续脱敏。
Web 用户仍只看到直接输入密钥；单次 CLI 子进程的安全传递方式保留。
Web 模型列表读取改为复用 provider_models，避免独立实现同一功能。

## 执行和撤销语义

按运行开始时加载的 Agent 快照授予工具，配置指纹包含 builtin_tools。
托管运行使用绑定身份；显式运行使用该 Agent 的固定 runtime 身份。
委派子 Agent 只使用自己的工具挂载，不继承父 Agent 的资源权限。
同 home 执行；工作目录不会改变资源管理范围。
无需启动外部 MCP；内置能力的显式挂载是权限来源。
模型需启用 tool_call。取消挂载从下一次运行生效，不取消已经开始的调用。
现有讨论结束／租约失效时阻止后续框架副作用的检查继续生效。
审计只记录参数形状和安全错误，不记录密钥、资源正文等原始参数。

## 验证标准

1. 旧 Agent YAML 缺省无内置权限，合法挂载可往返保存，未知／重复组拒绝。
2. 未挂载时伪造 framework_resource_write 不得写入；挂载后可完成资源 CRUD。
3. 资源权限可创建 Agent 身份、配置运行及启停，但不能创建普通用户身份；委派不继承权限，运行中修改不改变快照。
4. MCP 与 CLI 工具调用均支持直接密钥、模型列表、分类和静态校验；错误不泄漏密钥。
5. 删除默认资源、非空分类、仍托管 Agent 等继续被拒绝。
6. Web 创建／编辑可挂载和移除内置工具；取消编辑不落盘，保存失败保留表单。

## 创建并启用 Agent

1. `resource_write(kind=agent, create=true)` 保存配置，不自动运行。
2. 用 `principal_list` / `binding_list` 检查身份，`principal_create(kind=agent)` 创建新身份。
3. `agent_bind(principal, agent)` 创建绑定并默认启用：目录默认 home、模型调用轮数不限、
   policy 默认 mentioned；MCP 按 Agent 装配加载。更新已停用的绑定不会自动启用。
4. `agent_set_enabled(principal, enabled)` 保留设置启停；启用可能处理已授权的积压消息。
5. `agent_unbind` 删除运行设置，用于解除配置或删除 Agent 前清理，非普通停用。

内置运行时以上工具加 `framework_` 前缀。停用自己会终止自己的后续托管任务。

Agent 的 `resource_write` 返回配置摘要及 `activation`：configured、enabled、各身份的
bindings 和 next_steps。新配置没有绑定时明确返回未启用及创建身份/绑定步骤；不能只凭
资源创建成功就报告“已就绪”。`agent_bind` 返回 enabled 和 next_step；更新停用绑定
仍停用，模型需显式启用并确认对应身份的状态。

## Harness 管理工具

带有 `resource_management` 内置能力的 Lead 可以管理 Harness 资源并选择某个 Agent
在某个聊天中的模型。原生 Agent 与 Codex 托管 Agent 共用工具契约，不依赖 Web 弹窗。
CLI `tool list/call` 和 stdio MCP 使用同一个服务入口；本地管理入口仍需固定身份与管理权限。

### 建议调用顺序

1. `resource_write(kind="harness", create=true, ...)` 保存 Codex 连接配置。
   这不会安装 Codex，也不会创建容器；可执行文件和项目目录须已可用。
2. `harness_models(reference)` 读取该连接实际提供的模型和思考档位。
3. 创建引用 Harness 的 Agent，创建身份并 `agent_bind` 启用；先查现有绑定，避免重复。
4. `harness_session_get(conversation, target)` 获取当前执行会话。
5. `harness_session_models(conversation, target, session)` 获取当前会话配置快照对应的模型目录。
6. `harness_session_configure(conversation, target, session, model, effort?)` 修改该 Agent/聊天的设置。

`session` 是第 4 步返回的 AGT 记录 `id`；引擎的 `session_id` 仍由 Codex 生成。
修改 Harness 默认值仅影响新会话；修改会话不会更改共享资源或其他聊天。
省略 effort 时使用所选模型报告的默认档位，不推测可用模型或档位。

调用者必须拥有资源管理能力，调用者与目标必须是该活跃聊天的当前成员。
会话记录变化、正在执行、恢复中或需要人工核对时拒绝配置，不自动停止或重置。
被撤销的调度任务不能写入配置。模型发现不占用调度写锁，最终写入同时受任务校验与会话租约保护。

### 模块边界

- `agt-service/harness_tools` 定义工具契约与权限入口；通过 `HarnessControl` 注入实现。
- `agt-runtime/harness_control` 绑定调用身份、聊天范围和调度任务。
- `HarnessSessions` 负责会话配置、真实目录验证和执行锁；`agt-harness` 负责 Codex 协议。
- CLI/MCP、托管 Agent 和 Web 的模型目录/配置操作复用以上实现。

### 进度与隐私

工具调用的开始、完成和失败来自持久化调用审计；`tool_progress` 将其投影为最近 40 条活动。
原生 Agent 与 Codex 的框架工具都展示可读名称、具体工具名、状态和耗时。
摘要采用框架白名单，例如“找到 2 个模型”“会话模型已更新”；不展示参数正文、密钥、路径、
模型内部推理或工具返回正文。Codex 命令与文件活动保留类型提示，MCP 工具名以协议实际提供为准。

聊天横条默认折叠，展开显示近期操作及仍在执行的操作。任务详情和回复说明可查看活动记录；
回复说明展示首批调用中的最近 40 个工具，完整分页记录仍在调试模式中查看。
执行结束但缺少工具结束记录时显示“状态待确认”，不会误称仍在运行或已经成功。
Lead 后台查询和配置不打开 Web 弹窗，用户主动使用会话控制面板时才出现相应交互。
