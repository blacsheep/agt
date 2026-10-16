# 接入模型

Native Agent 通过两种资源访问模型：**Provider**（请求发往哪里）和 **Model**（在那里使用哪个模型、怎么调用）。

## Provider

Provider 是一个 OpenAI 兼容接口。OpenAI、API 网关以及 Ollama、vLLM 等本地服务都可以，只要它们实现了 OpenAI API。

**Web 控制台：** **资源库** → **新建资源** → Provider。填写名称、base URL 和 API Key。

**CLI：**

```bash
# 从环境变量读取密钥（密钥值会保存到 Provider 文件中）
agt provider create openai --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY

# 或通过管道传入，避免留在 shell 历史里
echo "$KEY" | agt provider create local --base-url http://localhost:11434/v1 --api-key-stdin

agt provider list-models openai     # 列出接口提供的模型
agt provider update openai --base-url https://gateway.example.com/v1
```

密钥保存在 home 中该 Provider 的 `provider.yaml` 里。agt 不会在列表、工具结果或错误中返回密钥，但它不是加密保险库：请像保护任何含凭据的文件一样保护 home 目录。

## Model

Model 资源指定 Provider 上的一个模型，以及 agt 如何调用它。

**Web 控制台：** **资源库** → **新建资源** → Model。选择 Provider，点击**获取模型列表**或填写模型名，并选择 API 格式。

**CLI：**

```bash
agt model create fast --provider openai --model gpt-5-mini \
  --api-format responses --tool-call true
agt model update fast --temperature 0.2 --timeout 120
agt model info fast
```

最重要的几个选项：

- `--api-format`：Provider 对这个模型使用的接口协议。agt 不会自动切换。

  | 格式 | 适用于 | Provider 的 Base URL |
  | --- | --- | --- |
  | `chat-completions`（默认） | OpenAI 及大多数兼容服务 | 例如 `https://api.openai.com/v1` |
  | `responses` | OpenAI Responses API | `https://api.openai.com/v1` |
  | `anthropic-messages` | Claude（Anthropic Messages API） | `https://api.anthropic.com/v1` |
  | `gemini` | Google Gemini API | `https://generativelanguage.googleapis.com/v1beta` |
- `--tool-call true`：使用任何工具的 Agent 都需要。
- `--reasoning-effort`：推理模型的思考档位。
- `--context-window`：模型接受的输入 token 数（默认 272000）。请按 Provider 文档设置；agt 据此判断何时摘要旧的上下文。

所有选项、默认值和文件格式见 [Model 模块](../modules/model.md)。

多个 Model 资源可以指向 Provider 上的同一个模型、使用不同设置，例如 `fast` 和 `careful` 两个版本。

## agt 如何调用模型

- **流式输出。** 请求使用流式模式，控制台可以在模型生成时显示进度和回复预览。忽略流式参数的 Provider 也能用，只是没有预览。
- **重试。** 连接错误、服务端错误、无效响应和空回复会自动重试，最多 10 次，间隔逐步增加。配置错误或模型明确拒绝等错误不会重试。
- **上下文管理。** 对话接近上下文窗口的 90% 时，agt 会自动摘要较早的历史（这会调用模型）。可以在会话中用 `/new` 重置 Agent 的上下文，聊天记录会保留。
- **仅文本。** 暂不支持图片输入。

## 检查连接

```bash
agt provider test openai          # 校验配置（不访问网络）
agt provider list-models openai   # 用保存的密钥调用 Provider 的 /models 接口
```

最可靠的测试是和使用该模型的 Agent 简短私聊一次。如果失败，在控制台打开调试模式，或运行 `agt run info <执行 ID>` 查看 HTTP 状态码和尝试次数。见[常见问题与排错](../help/troubleshooting.md)。
