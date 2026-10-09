# Agent 开发和装配 MCP 工具

复用 MCP 资源仓库与 `Agent.mcps`；工具实现代码及依赖由开发任务准备，
注册仅保存启动/连接配置，不安装依赖，不部署服务，也不启用 Agent。

## 开发契约

开发任务应交付：服务源码、启动方式、依赖安装说明，以及本地契约测试。
优先使用 MCP SDK 的类型定义同时生成 `inputSchema` 和服务端参数解析器，
不要分别手写两份容易漂移的参数规则。测试必须实际调用服务：至少包含合法参数、
缺少必填字段、错误类型、边界值和业务错误。客户端 schema 校验不能证明实现与声明一致。
服务端仍须验证业务状态与权限；schema 合法并不代表调用被授权或业务一定成功。

## 注册、探测与装配

1. `resource_write(kind="mcp", reference=..., create=true, data={mcpServers:{...}})`
   注册单个服务；亦可使用 `agt mcp create`。
2. `resource_test(kind="mcp", reference=...)` 只验证连接配置，不启动服务。
3. `mcp_probe(reference=..., working_directory=...)` 显式启动/连接服务，执行
   initialize、tools/list 并编译每个 inputSchema，随后关闭连接；不调用任何业务工具。
   CLI 对应 `agt mcp probe <reference> --working-directory <directory>`。
4. 使用既有 `resource_write(kind="agent", reference=..., create=false, data={mcps:[...]})`
   设置 MCP 引用列表；提供 `mcps` 时替换整个列表，省略时保持原值，空列表清空挂载。
   `agent_mcp_list` 查看配置。引用支持 category/name，不新增挂载表。
   CLI 的 `agt agent update <agent> --add-mcp/--remove-mcp <mcp>` 则增删单项。
   两条路径共用 Store 的文件锁与引用校验，但全量列表替换不能避免旧快照覆盖新配置。
5. 检查 Agent 的身份、绑定启用状态与 binding/discussion 的 MCP 授权，然后派发新任务。

这些 Framework 工具只向 manager 或安装 `resource_management` 的 Agent 暴露，
Native 中也经过现有权限策略。宿主必须另外允许 probe：托管 Native 使用 binding/discussion
计算后的 allow_mcp；Harness 默认不暴露 AGT probe。直接管理端可显式探测。
probe 会执行配置的启动命令或访问 URL，不是静态检查。
stdio 服务运行在宿主机上，注册和探测本身不提供沙箱。

## Run 内的工具契约

Native 每次新 Run 连接 MCP 并读取 tools/list，将工具 schema 与已编译 validator
保存在同一份 Run 快照。schema 也参与已有恢复指纹；探测结果不是运行时的永久认证。
Framework 和 delegate 同样使用 Run 的 schema 快照。

处理顺序：批次结构/名称/身份检查 → 参数解析与校验 → 权限审核 → before → execute → after。
参数不符合 schema 时，不创建审批、不调用后端；记录失败工具结果（invalid_arguments），
保留 tool_call_id 并返回模型，由现有模型步数预算约束修正过程。没有独立重试循环。
参数错误不阻止批次内其他合法调用；批次身份、未知工具及委派授权错误仍保留原有拒绝语义。

共享 `agt_core::tool_schema` 使用 jsonschema 标准校验器：

- 默认 JSON Schema 2020-12，显式标准 dialect 由库识别；不支持的 dialect 加载失败。
- 递归类型、required、enum/const、数组、范围、长度、组合约束、本地引用均按 schema 检查。
- 工具参数必须为对象；显式声明为非对象的根类型在加载时拒绝。
- 不转换类型，不插入默认值；additionalProperties 遵循服务声明，不擅自收紧。
- 禁止 schema 编译时下载或读取外部引用；引用需要包含在 schema 内。
- format 遵循 dialect 的标准 annotation/assertion 语义，不把所有格式注释强制变为业务约束。
- 无效 schema 在加载/探测阶段失败；参数错误反馈字段与 schema 路径，不回显参数值。

Framework 服务入口也使用同一校验器，并保留原有 ToolError 顶层字段格式和后续
typed/domain 校验。Harness 的 AGT 动态工具在提交回合前编译定义，收到调用后先验证，
通过才进入宿主回调。Codex 自有命令、文件或 MCP 工具仍由 Codex 管理。

装配仅影响新 Run；当前 Run 的工具快照不热更新。Harness Agent 暂不支持 AGT MCP 挂载。
输出 schema 校验、工具安装器、运行时热装载不在本次范围。

## 本地验证

`cargo test -p agt-core tool_schema` 检查标准约束与离线引用；
`cargo test -p agt-service --test service assembly` 检查配置替换、入口权限、已删除工具不可用及并发更新；
`cargo test -p agt-runtime --test runtime tool_contract` 使用本地 Python MCP 服务验证
注册→探测→挂载→错误调用→修正→卸载及新 Run 生效。测试不调用真实模型。
