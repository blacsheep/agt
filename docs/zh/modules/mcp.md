# MCP

| 概览 | |
| --- | --- |
| **是什么** | 为 Agent 提供工具的 MCP 服务 |
| **CLI** | `agt mcp` |
| **Agent 工具** | `kind: "mcp"` 的 `resource_*`、`resource_test`、`mcp_probe`、`agent_mcp_list` |
| **Web 控制台** | **资源库** → MCP；Agent 表单中的 MCP |
| **文件** | `mcps/<category>/<name>/mcp.json` |
| **Crate** | `agt-mcp` |

## 作用

MCP 资源描述如何启动（stdio）或连接（HTTP）一个 [Model Context Protocol](https://modelcontextprotocol.io) 服务。
把它挂载到 Native Agent 上，Agent 就能调用它的工具。每次执行开始时，Agent 会连接自己的 MCP 服务；
每次调用都会按工具的输入 schema 和[审批策略](./permissions.md)检查。

注册服务不会让任何人获得使用权；只有挂载了它的 Agent 才能使用。Codex Agent 不使用这些资源，请在 Codex 中配置 MCP。

## 怎么用

**Web 控制台：** **资源库** → **新建资源** → MCP，然后粘贴配置。

**CLI：**

```bash
agt mcp create files --file ./files.json
agt mcp probe files                       # 启动服务并列出工具
agt agent update researcher --add-mcp files
```

**Agent 工具：** 用 `resource_write` 注册，用 `mcp_probe` 检查，用
`resource_write(kind="agent", data={}, add={"mcps":["files"]})` 挂载。见 [MCP 工具](../guide/mcp.md#让-agent-自己造工具)。

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `command`、`args`、`cwd` | 如何启动 stdio 服务；`cwd` 必须是绝对路径 |
| `url` | HTTP 服务的地址 |
| `bearer_token_env_var` | 保存 HTTP bearer token 的环境变量 |

## 全部参数

### 命令

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `create <名称>` | `--file <路径>`（必填）、`--category <c>` | 从 JSON 文件注册服务 |
| `update <名称>` | `--file <路径>`（必填）、`--category <c>` | 替换配置 |
| `test <引用>` | `--category <c>` | 校验配置。不启动也不连接服务。 |
| `probe <引用>` | `--category <c>` | 按真实执行的方式启动或连接，完成初始化，列出工具并编译其输入 schema，然后断开。不调用任何工具。 |
| `list`、`info`、`delete`、`category` | | 见[资源通用操作](./resources.md#通用-cli-命令) |

### 文件：`mcp.json`

文件是一个只包含一个服务的 `mcpServers` 对象，服务的键名必须等于资源名。

**stdio 服务**

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

| 字段 | 必填 | 含义 |
| --- | --- | --- |
| `command` | 是 | `PATH` 中的程序名，或绝对路径 |
| `args` | 否 | 字符串数组。相对路径按 `cwd` 解析；`${cwd}` 和 `${workspace}` 会被替换为 `cwd`。 |
| `cwd` | 是 | 绝对工作目录。无论哪个 Agent 使用，服务总在这里启动。 |
| `env` | 否 | 额外的环境变量（字符串值） |
| `type` / `transport` | 否 | `stdio`。省略时自动推断。 |

**HTTP 服务**

```json
{
  "mcpServers": {
    "search": {
      "url": "https://mcp.example.com/mcp",
      "bearer_token_env_var": "SEARCH_TOKEN",
      "http_headers": { "X-Team": "research" },
      "env_http_headers": { "X-Account": "SEARCH_ACCOUNT" }
    }
  }
}
```

| 字段 | 必填 | 含义 |
| --- | --- | --- |
| `url` | 是 | 绝对的 `http` 或 `https` URL，不能包含用户名或密码 |
| `bearer_token_env_var` | 否 | 环境变量名，其值作为 `Authorization: Bearer ...` 发送 |
| `http_headers` | 否 | 固定请求头 |
| `env_http_headers` | 否 | 值来自环境变量的请求头（请求头名 → 变量名） |
| `type` / `transport` | 否 | `http`。根据 `url` 自动推断。 |

环境变量在 agt 连接时读取，来自 Web 控制台或调度器进程的环境。

### Agent 工具

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `resource_write` | `kind: "mcp"`, `data: {mcpServers: {...}}` | 注册或替换服务 |
| `resource_test` | `kind: "mcp"`, `reference` | 只校验配置 |
| `mcp_probe` | `reference` | 启动或连接并检查工具，同 `agt mcp probe` |
| `agent_mcp_list` | `agent` | 列出挂载到某个 Agent 的 MCP 服务 |

### 工具调用如何检查

- 工具参数首先按服务的输入 schema（默认 JSON Schema 2020-12）校验。无效调用会把错误返回给模型，不会到达服务或审批队列。
- schema 不能加载外部引用。
- 拥有 `resource_management` 的 Agent 修改自己的 MCP 服务时，新工具在当前这批工具调用结束后加载。加载失败时保留旧工具，并告知原因。

## 相关模块

- [Agent](./agent.md)：挂载服务
- [权限审批](./permissions.md)：每次调用的审批
- [Harness](./harness.md)：使用自身 MCP 配置的 Codex Agent
