# MCP 工具

[Model Context Protocol](https://modelcontextprotocol.io) 服务为 Agent 提供工具：文件访问、网页搜索、数据库、游戏状态，或任何你自己写的功能。在 agt 中，MCP 服务是一种资源，可以挂载到 Native Agent 上。

## 注册服务

MCP 资源是一个只包含一个服务的 `mcpServers` 文档，格式与许多 MCP 客户端相同。

**stdio 服务**（由 agt 启动进程）：

```json
{
  "mcpServers": {
    "files": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/srv/shared"],
      "cwd": "/srv/shared",
      "env": { "LOG_LEVEL": "warn" }
    }
  }
}
```

- `cwd` 必须是绝对路径。无论哪个 Agent 使用，服务总是在这里启动。
- `command` 是 `PATH` 中的命令名或绝对路径。`args` 中的相对路径按 `cwd` 解析，`${cwd}` 会被替换为它。

**HTTP 服务**（agt 连接已在运行的服务）：

```json
{
  "mcpServers": {
    "search": {
      "url": "https://mcp.example.com/mcp",
      "bearer_token_env_var": "SEARCH_TOKEN",
      "http_headers": { "X-Team": "research" }
    }
  }
}
```

凭据不要写在 URL 里；使用 `bearer_token_env_var`、`http_headers` 或 `env_http_headers`（请求头名到环境变量名的映射）。环境变量在 agt 连接时读取，因此要在启动 Web 控制台或调度器的环境中设置。

**Web 控制台：** **资源库** → **新建资源** → MCP，然后粘贴文档。

**CLI：**

```bash
agt mcp create files --file ./files.json
agt mcp update files --file ./files.json
```

## 检查能否使用

| 命令 | 作用 |
| --- | --- |
| `agt mcp test files` | 只校验配置，不启动服务。 |
| `agt mcp probe files` | 按与真实执行完全相同的方式启动或连接服务，列出工具并检查其输入 schema。不调用任何工具。 |

探测成功说明 Agent 能连上并看到这些工具。工具行为是否正确，要靠服务自己的测试保证。

## 挂载到 Agent

```bash
agt agent update researcher --add-mcp files
agt agent update researcher --remove-mcp files
```

也可以在 Agent 的**编辑角色**表单的 MCP 部分勾选。Agent 在每次执行开始时连接其 MCP 服务，所以修改从下一次执行开始生效。拥有 `resource_management` 的 Agent 修改自己的 MCP 服务时，改动在当前这批工具调用结束后立即生效。

Agent 只能使用挂载给它的服务。在 home 中注册服务并不会让任何人获得使用权。

## 让 Agent 自己造工具

拥有 `resource_management` 的 Agent 可以通过工具完成上述所有操作：用 `resource_write(kind="mcp")` 注册服务，用 `resource_test` 和 `mcp_probe` 检查，再用 `resource_write(kind="agent", create=false, data={}, add={mcps:[...]})` 挂载。配合一个编写服务代码、跑在 [Codex](./codex.md)、[Claude Code](./claude-code.md) 或 [DeepSeek Harness](./deepseek-harness.md) 上的编程 Agent，lead 就能造出团队缺少的工具。见[让 lead 组建团队](./lead-team.md)。

让 Agent 编写 MCP 服务时，建议要求：

- 输入 schema 由服务解析参数所用的同一套类型生成；
- 契约测试实际调用服务，覆盖合法输入、缺少字段、类型错误、边界值和业务错误；
- 启动命令在服务所在目录中可以直接运行。

## 工具调用如何校验

Agent 的每次调用在执行任何操作前，都会先按工具的输入 schema 校验。参数无效时，错误会返回给模型让它修正，不会到达服务，也不会进入审批队列。有效的调用再经过[审批策略](./permissions.md)。

## 限制

- stdio 服务在宿主机上以运行 agt 的用户权限运行，agt 不对其做沙箱隔离。只挂载你信任的服务。
- 已经发给服务的请求无法撤回，即使你停止了讨论。
- Harness Agent（Codex、Claude Code、DeepSeek Harness）不使用 agt 的 MCP 资源；请在 Harness 自己的配置中设置 MCP 服务。
