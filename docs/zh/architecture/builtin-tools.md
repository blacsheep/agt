# 内置工具：权限与契约

工具列表与参数见[工具服务模块](../modules/tool-service.md#可用的工具)。本文记录资源管理工具的授权范围、契约与语义，供贡献者参考。

内置工具按组挂载到 Agent 定义的 `builtin_tools`，默认全部关闭，由操作员显式开启。Web 在创建/编辑 Agent 时勾选，CLI 用 `--builtin-tool`、`--add-builtin-tool`、`--remove-builtin-tool`。模型侧的工具名带 `framework_` 前缀。

| 工具组 | 用途 | 说明 |
| --- | --- | --- |
| `resource_management` | 管理资源、Agent 身份与绑定、Harness 会话 | 本文 |
| `task_collaboration` | 委派、等待、汇报显式工作任务 | [异步任务协作](tasks.md) |
| `local_execution` | 在宿主机执行命令（还需 `execution.enabled`） | [运行配置与记忆](config-and-memory.md#本机执行) |

消息收发、会话管理和 Memory 工具对所有托管 Agent 固定开放，不需要挂载。

以下各节说明 `resource_management`。

## Agent 授权范围

Agent 定义增加 `builtin_tools: [resource_management]`，缺省为空。Web 创建／编辑 Agent 提供“资源管理”复选框，操作员勾选并保存即授予权限。CLI 提供 `--builtin-tool`、`--add-builtin-tool`、`--remove-builtin-tool`。未知工具组、配置文件及创建请求中的重复挂载拒绝保存；CLI 添加已挂载的组采用幂等行为。

授权作用于当前框架 home 中的全部七类资源，不局限于该 Agent 自身的挂载资源。可以读取、创建、修改、删除 Provider、Model、Prompt、Skill、MCP、Harness 和 Agent 定义；因此也能修改其他 Agent 的后续配置，包括其内置资源工具挂载。不会影响已加载的运行快照。不把执行身份升级为 Manager。允许 principal_create（仅 Agent 类型）、agent_bind、agent_set_enabled、agent_unbind、binding_list；普通用户等身份创建仍仅 Manager 可用。不绕过通信工具的会话成员权限。

删除继续使用现有“计划 → 精确确认”协议；资源工具允许请求递归删除依赖者。默认 Prompt、仍托管的 Agent、损坏配置和悬空引用继续受到现有删除保护。分类删除仅允许空的非 default 分类。模型可以自行完成两次工具调用，该确认协议不是 Web 人工审批弹窗；挂载资源管理意味着授权这些写入和删除能力。

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

MCP 和 CLI `tool call` 继续由 `--manager` 开放这组工具；内置挂载通过单独资源权限开放同一组，运行时名称添加 `framework_` 前缀。

Provider 创建支持 `{base_url, api_key}` 直接输入；保留 api_key_env 兼容旧 MCP 客户端，二者互斥，创建必填一种非空密钥，更新可省略密钥。所有返回与错误继续脱敏。Web 用户仍只看到直接输入密钥；单次 CLI 子进程的安全传递方式保留。Web 模型列表读取改为复用 provider_models，避免独立实现同一功能。

## 执行和撤销语义

按运行开始时加载的 Agent 快照授予工具，配置指纹包含 builtin_tools。托管运行使用绑定身份；显式运行使用该 Agent 的固定 runtime 身份。委派子 Agent 只使用自己的工具挂载，不继承父 Agent 的资源权限。同 home 执行；工作目录不会改变资源管理范围。无需启动外部 MCP；内置能力的显式挂载是权限来源。模型需启用 tool_call。取消挂载从下一次运行生效，不取消已经开始的调用。现有讨论结束／租约失效时阻止后续框架副作用的检查继续生效。审计只记录参数形状和安全错误，不记录密钥、资源正文等原始参数。

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
3. `agent_bind(principal, agent)` 创建绑定并默认启用：目录默认 home、模型调用轮数不限、policy 默认 mentioned；MCP 按 Agent 装配加载。更新已停用的绑定不会自动启用。
4. `agent_set_enabled(principal, enabled)` 保留设置启停；启用可能处理已授权的积压消息。
5. `agent_unbind` 删除运行设置，用于解除配置或删除 Agent 前清理，非普通停用。

内置运行时以上工具加 `framework_` 前缀。停用自己会终止自己的后续托管任务。

Agent 的 `resource_write` 返回配置摘要及 `activation`：configured、enabled、各身份的 bindings 和 next_steps。新配置没有绑定时明确返回未启用及创建身份/绑定步骤；不能只凭资源创建成功就报告“已就绪”。`agent_bind` 返回 enabled 和 next_step；更新停用绑定仍停用，模型需显式启用并确认对应身份的状态。

