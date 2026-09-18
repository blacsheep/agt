# Harness 管理工具与执行进度

带有 `resource_management` 内置能力的 Lead 可以管理 Harness 资源并选择某个 Agent
在某个聊天中的模型。原生 Agent 与 Codex 托管 Agent 共用工具契约，不依赖 Web 弹窗。
CLI `tool list/call` 和 stdio MCP 使用同一个服务入口；本地管理入口仍需固定身份与管理权限。

## 建议调用顺序

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

## 模块边界

- `agt-service/harness_tools` 定义工具契约与权限入口；通过 `HarnessControl` 注入实现。
- `agt-runtime/harness_control` 绑定调用身份、聊天范围和调度任务。
- `HarnessSessions` 负责会话配置、真实目录验证和执行锁；`agt-harness` 负责 Codex 协议。
- CLI/MCP、托管 Agent 和 Web 的模型目录/配置操作复用以上实现。

## 进度与隐私

工具调用的开始、完成和失败来自持久化调用审计；`tool_progress` 将其投影为最近 40 条活动。
原生 Agent 与 Codex 的框架工具都展示可读名称、具体工具名、状态和耗时。
摘要采用框架白名单，例如“找到 2 个模型”“会话模型已更新”；不展示参数正文、密钥、路径、
模型内部推理或工具返回正文。Codex 命令与文件活动保留类型提示，MCP 工具名以协议实际提供为准。

聊天横条默认折叠，展开显示近期操作及仍在执行的操作。任务详情和回复说明可查看活动记录；
回复说明展示首批调用中的最近 40 个工具，完整分页记录仍在调试模式中查看。
执行结束但缺少工具结束记录时显示“状态待确认”，不会误称仍在运行或已经成功。
Lead 后台查询和配置不打开 Web 弹窗，用户主动使用会话控制面板时才出现相应交互。
