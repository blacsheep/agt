# 资源管理与能力生效

最近代码核对：2026-10-09。此页描述已实现行为，覆盖 Agent、Model、Provider、
Prompt、Skill、MCP、Harness 七类配置资源。身份绑定、消息、Run 和 WorkItem
继续由各自业务模块管理。

## 共享资源管理入口

```mermaid
flowchart TD
    CLI["agt 资源命令<br/>解析参数、文件或 stdin"] --> Resources
    Web["Web 资源请求"] --> Bridge["Node → agt tool call"]
    Bridge --> Tools["ToolService<br/>固定身份、能力与参数检查"]
    Model["模型 framework_resource_* 调用"] --> Gate["Runtime 工具校验与执行控制"]
    Gate --> Tools
    Tools --> Resources["Resources / agt-service<br/>统一资源操作"]
    Resources --> Read["list / read<br/>安全视图；read 返回 revision"]
    Resources --> Write["create / update<br/>共享写锁内应用修改"]
    Resources --> Delete["删除计划与执行<br/>引用、绑定与确认检查"]
    Resources --> Category["分类列表、创建、空分类删除"]
    Write --> Version{"expected_revision<br/>若提供，是否匹配？"}
    Version -->|否| Conflict["返回 conflict，不修改"]
    Version -->|是或未提供| Domain["对应资源写入模块<br/>类型解析、更新规则与领域校验"]
    Domain --> Store["对应 Store<br/>原子写入或暂存提交"]
    Store --> Result["返回安全视图与新 revision"]
```

CLI 和 ToolService 调用同一个资源服务，不在入口层分别修改配置。
七类资源按模块分开，保留 Skill 文档/文件包、MCP 连接配置、Provider 凭据等差异。
CLI 可投影结果以保持既有 info/read 输出形状。

- `data` 保留原有更新语义；Agent 集合字段出现时整体替换，省略则保持。
- `add/remove` 支持 Agent 的 skills、mcps、builtin_tools；新增去重，移除不存在项失败。
  同字段的替换/清空不能与集合增删混用，同一规范化引用不能同时增删。
- `clear` 只支持明确允许清空的字段，不提供任意 JSON 路径修改。
- `revision` 是持久化内容指纹，不是递增计数；`expected_revision` 可选。
  Web Agent/Harness 编辑携带读取时的版本，旧客户端不传版本时保留既有覆盖语义。
- `.resources.lock` 覆盖管理读写、分类修改和删除执行；联网查询与探测先在锁内读取配置，
  再在锁外使用该快照执行。外部手工文件编辑不参与这把锁。
- 删除复用依赖图、显式计划确认及绑定检查。文件与 IM/绑定数据库不构成全局事务。
- 保存配置不自动连接 MCP、安装依赖或启用 Agent。MCP 探测是独立操作。

完整字段契约见[共享资源管理](resources.md)。

## Native 自身能力刷新

触发条件是当前 Native Run 成功调用 `framework_resource_write`，修改自身 MCP/Skill
挂载，或者修改当前已挂载的 MCP/Skill 资源。多个符合条件的写入合并到批次结束时刷新一次。

```mermaid
sequenceDiagram
    participant M as 模型
    participant N as Native 工具批次
    participant R as Resources
    participant C as 候选能力加载
    M->>N: resource_write 修改自身能力
    N->>R: 权限通过后保存配置
    R-->>N: 保存成功与 revision
    N->>N: 记录 runtime_application=pending
    N->>N: 用旧能力完成本批次剩余调用
    N->>C: 批次结束，读取 MCP/Skill 候选配置
    C->>C: 保留当前模型、Provider、基础 Prompt 和内置能力
    C->>C: 检查模型工具能力，按 Agent 装配加载 MCP
    opt MCP 配置或挂载变化
        C->>C: 连接服务、获取 schema、检查工具数量并编译校验器
    end
    alt 加载成功且取消/执行资格检查通过
        C-->>N: 候选提示词与可选工具快照
        N->>N: 更新执行指纹，切换能力
        N->>M: 写入结果 + 独立 applied 通知 + 当前工具 schema
    else 加载失败
        C-->>N: 失败原因
        N->>N: 保留旧能力，记录失败审计；不撤销已保存配置
        N->>M: 写入结果 + 独立 failed 通知 + 旧工具 schema
    else 取消或失去执行资格
        C-->>N: 终止刷新与执行
    end
```

Skill 单独变化不会重启现有 MCP 连接。MCP 变化时，候选连接、工具 schema、校验器和
执行路由准备完成后才切换；工具别名由资源引用及远端工具名决定，挂载重排不会串到另一服务。
模型可以从刷新后的下一次请求开始使用新工具，批次内不会改变能力。

刷新结果写入调用审计，并作为独立通知加入模型上下文。加载失败不会自动反复重试。
成功刷新的执行指纹进入挂起检查点；恢复仍会重新装配并核对指纹与工作目录，不重放已完成工具。

## 生效范围

| 修改来源或内容 | 当前行为 |
| --- | --- |
| Native Run 成功修改自身 MCP/Skill | 本工具批次完成后刷新，加载成功才对当前 Run 生效 |
| Native Run 修改另一个 Agent | 该 Agent 的后续新 Run 加载，不刷新它当前执行 |
| Web/CLI 修改配置 | 后续新 Run 加载，不向当前 Run 注入刷新 |
| Model、Provider、基础 Prompt、内置能力、委派配置 | 不随 MCP/Skill 刷新改变当前 Run |
| Harness 执行 | 不使用 Native 能力刷新；AGT MCP 挂载仍不支持，已有会话保留其配置语义 |

管理员实时切换审批策略的既有行为独立保留。能力刷新不会扩大 Framework 能力。
MCP 使用范围由 Agent 装配决定，未装配的工具不可调用；具体调用仍经过权限审批，
没有绑定或 Discussion 的 MCP 总开关。资源的 `revision` 与 Run 的 `config_fingerprint`
也不是同一个标识：前者检测资源编辑冲突，后者检查执行及恢复配置。

## 代码与验证入口

- [资源服务](../../crates/agt-service/src/resources/mod.rs)与[更新/版本检查](../../crates/agt-service/src/resources/mutation.rs)。
- [CLI 适配](../../crates/agt-cli/src/main.rs)与[工具协议](../../crates/agt-service/src/tools.rs)。
- [删除依赖与执行](../../crates/agt-service/src/deletion.rs)。
- [批次执行](../../crates/agt-runtime/src/engine/native/tools.rs)与[能力刷新](../../crates/agt-runtime/src/engine/native/refresh.rs)。
- [配置解析](../../crates/agt-inference/src/resolve.rs)与[MCP 快照](../../crates/agt-runtime/src/engine/native/mcp.rs)。
- [资源回归](../../crates/agt-service/tests/service/assembly.rs)：七类版本冲突、集合并发与失败原子性。
- [运行时回归](../../crates/agt-runtime/tests/runtime/tool_contract.rs)：自装配、失败保留、授权与工具别名。
- [挂起恢复回归](../../crates/agt-runtime/src/scheduler/tests/wait_tests.rs)：刷新后恢复且不重放配置写入。

回归使用本地模型和 MCP 夹具，不调用真实模型。
返回[架构图导航](README.md)。
