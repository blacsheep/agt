# DeepSeek Harness Agent

DeepSeek Harness Agent 运行在 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness)（`dsh`）上，而不是 agt 自己的模型循环。agt 通过 [Agent Client Protocol](https://agentclientprotocol.com)（ACP）驱动它，也就是编辑器对接 Agent 用的那套开放协议。它保留自己的工具、插件和沙箱，同时可以使用 agt 的消息、记忆和任务工具。

## 前提

- 运行 agt 的机器上已经安装 DeepSeek Harness，例如 `npm install -g @deepseek-ai/dsh`。它需要 Node.js 24 或更高版本，并且要在 agt 进程的 `PATH` 里。
- DeepSeek 的 API key 已经保存在 DeepSeek Harness 里（它的 Web 界面的 Models 页），或者在运行 agt 的环境里导出了 `DEEPSEEK_API_KEY`。agt 不保存 DeepSeek 的凭据。

agt 每一轮启动一次 `dsh --profile acp`，下一轮接着用这个 Agent 的会话。

## 创建 Harness

```json
{
  "engine": "deepseek-harness",
  "executable": "/usr/local/bin/dsh",
  "working_directory": "/home/me/projects/tools",
  "sandbox": "workspace_write",
  "model": "deepseek-v4-pro",
  "reasoning_effort": "high",
  "timeout_seconds": 1800,
  "description": "为团队开发工具"
}
```

| 字段 | 对 DeepSeek Harness 的含义 |
| --- | --- |
| `model` | DeepSeek Harness 提供的模型，写名称（`deepseek-v4-pro`）或它的选项值都可以。留空沿用 DeepSeek Harness 的默认模型。写了不存在的名称时，这一轮会失败并列出可用的名称。 |
| `reasoning_effort` | DeepSeek Harness 提供的档位：`off`、`low`、`high` 或 `max`。 |
| `sandbox` | 为了和其他引擎保持一致而保留；DeepSeek Harness 使用它自己的权限预设。 |

**Web 控制台：** **资源库** → **新建资源** → Harness，执行引擎选择 **DeepSeek Harness**。

**CLI：**

```bash
agt harness resource create builder --file ./builder.json
agt harness resource test builder      # 启动 dsh 并检查 ACP 握手，不调用模型
```

然后在它上面创建 Agent：`agt agent create builder --harness builder --prompt builder`。

## agt 怎么和 DeepSeek Harness 对接

- **agt 工具：** 每一轮 agt 在 `127.0.0.1` 上临时开一个带随机令牌的 MCP 端点，作为名为 `agt` 的服务挂载到 ACP 会话里。工具调用在 agt 内部执行，照常做权限检查。
- **审批：** 权限请求（`session/request_permission`）进入 agt 的审批队列，按 **Harness** 审批模式处理，见[权限审批](./permissions.md#codex-审批)。可以批准本次、拒绝或取消这一轮。
- **模型和档位：** 通过 ACP 的会话配置项逐个会话设置。
- **会话：** 每个 Agent 在每个会话里保留一个会话；agt 每轮结束后关闭它，下一个进程才能接着用。`/new` 开始新会话。

## DeepSeek Harness Agent 能用和不能用的

| 可用 | 不可用 |
| --- | --- |
| 角色 Prompt 和 Skills | agt 的 MCP 资源（请在 DeepSeek Harness 里安装插件或配置 MCP） |
| `resource_management` 和 `task_collaboration` 工具 | `local_execution` |
| 消息、记忆和任务工具 | 响应策略 `all`（只支持 `mentioned` 和 `manual`） |
| `/stop`、`/new`、`/status`，回复实时预览 | `/model`、`/compact`、`/rename`、`/skills`、`/mcp`、`/permissions` |

## 注意事项

- **开发者预览版。** DeepSeek Harness 说明了会有不兼容的变化；agt 只使用 ACP v1 的标准方法。
- **使用同一工作目录的 Agent 会共享文件。**
- **停止不等于撤销。** 取消一轮会停下 DeepSeek Harness，但已经做出的修改会保留。
