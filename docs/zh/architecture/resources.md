# 共享资源管理

最近整理：2026-10-10（由英文版翻译）。流程图见[资源管理与能力生效](resource-flow.md)。

七类定义资源（Agent、Model、Provider、Prompt、Skill、MCP、Harness）共用 `agt_service::Resources`。 CLI 命令负责解析参数、文件或 stdin，并把结果投影为原有输出格式。框架工具和 Web 请求在完成身份与能力校验后调用同一服务。 Web 仍通过 CLI 调用 Rust，Node 中没有第二套资源实现。

`resources/mod.rs` 负责路由操作，七个资源模块各自负责写入契约。各领域 crate 保留配置校验和 Store 持久化； Skill 文档替换由 SkillStore 提供。分类修改和带依赖检查的删除使用同一把管理锁。

## 更新契约

`resource_write` 保持 `kind/reference/create/data` 接口：

- 省略的可选 data 字段保留原值。
- Agent 的 `data.skills/mcps/builtin_tools` 整体替换对应集合。
- 更新时，`add` 和 `remove` 可用于这三个 Agent 集合：引用会规范化，添加自动去重，移除不存在的项会失败。
- 同一字段不能同时整体替换/清空又做集合操作；同一规范化项也不能既添加又移除。
- `clear` 支持 Agent 的 description 与三个集合、Model 的 max_tokens 和 reasoning_effort、Harness 的 model 和 reasoning_effort。
- 其他资源收到集合操作会拒绝，而不是忽略。
- Model 原有的 `clear_*` 标志行为不变。
- 创建时拒绝所有更新修饰符。

示例：

```json
{
  "kind": "agent",
  "reference": "default/searcher",
  "create": false,
  "data": {},
  "add": {"mcps": ["default/search"]}
}
```

读写结果都带不透明的内容 `revision`。更新可选传入 `expected_revision`，不一致时在修改前返回 `conflict`。 Web 的 Agent/Harness 编辑器会回传读取时的版本；不传版本的旧客户端仍是“最后写入生效”。

home 级的 `.resources.lock` 串行化管理读、写、分类修改和删除执行；版本比较、合并和提交都在锁内完成。原有的原子写入和 MCP 暂存机制保留。内容版本覆盖未脱敏的持久化数据和 Skill 附属文件，但只返回哈希，不返回凭据。直接在外部编辑文件不参与这把锁，版本只能发现检查前已发生的修改。底层领域 Store 是持久化 API，不是另一个管理入口。

删除保留原有依赖图、明确的计划确认、已绑定 Agent 保护和身份清理。资源文件与 IM、绑定数据库仍是不同的事务域。

## 运行时生效

CRUD 只提交配置。Native Runtime 在成功修改自身 MCP/Skill 后，于工具批次边界单独刷新能力，并与写入结果分开报告“已生效”或“加载失败”。其他正在运行的 Agent、Model/Provider/基础 Prompt 的修改以及 Harness 会话保持各自原有的生效语义。详见 [MCP 工具](mcp-tools.md)。

导入格式、远端模型发现和 MCP 探测保留各资源专用的操作。保存定义不会探测服务、安装依赖，也不会启用 Agent 身份。
