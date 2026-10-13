# 在 MCP 客户端中使用 agt

`agt serve` 通过 MCP stdio 开放 agt 的工具，任何 MCP 客户端（编辑器、桌面助手或其他 Agent 框架）都能借此管理资源、参与会话。

## 配置客户端

大多数客户端接受一个 `mcpServers` 条目：

```json
{
  "mcpServers": {
    "agt": {
      "command": "/absolute/path/to/agt",
      "args": ["serve", "--principal", "me", "--manager"]
    }
  }
}
```

- `--principal` 设置客户端使用的身份。该身份必须已经存在（`agt im principal create me --name Me`）。模型无法通过工具参数修改它。
- `--manager` 增加资源和绑定工具。不加则客户端只有参与者工具。
- 在 `serve` 前加 `--home <目录>` 可以使用 `$AGT_HOME` 或 `~/.agt` 以外的 home。

## 客户端能做什么

不带 `--manager` 时，客户端只有参与者工具：会话、消息、记忆以及它自己的任务。带 `--manager` 时还有资源、身份和绑定工具。
完整列表见[工具服务模块](../modules/tool-service.md#可用的工具)。

`agt tool list --principal me --manager` 会打印每个工具及其参数。

## 注意事项

- **`agt serve` 不会运行 Agent。** 通过它发送的消息会被保存和投递，但只有在讨论已授权、调度器在运行时，Agent 才会回复。
  请在 Web 控制台中或用 `agt schedule start` 开始讨论，并保持控制台或 `agt schedule run --watch` 运行。
- **信任。** 启动 `agt serve` 的人决定使用哪个身份、是否为管理者。请把它当作本地管理工具对待。
- **规则与 Agent 相同。** 删除会先返回计划并需要精确确认，密钥永远不会被返回。

## 在脚本中调用工具

不需要 MCP 客户端也能使用同样的工具：

```bash
agt tool list --principal me --manager
agt tool call --principal me --manager resource_list --args '{"kind":"agent"}'
agt --output json tool call --principal me message_history --args '{"conversation":"<id>"}'
```

要响应变化，可以用 `agt events watch --principal me`：该身份可见的内容发生变化时（例如新消息或任务更新），它会输出一行 JSON。
