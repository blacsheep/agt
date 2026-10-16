# Provider

| 概览 | |
| --- | --- |
| **是什么** | 一个 OpenAI 兼容接口及访问它的 API Key |
| **CLI** | `agt provider` |
| **Agent 工具** | `kind: "provider"` 的 `resource_*`、`provider_models`、`resource_test` |
| **Web 控制台** | **资源库** → Provider |
| **文件** | `providers/<category>/<name>/provider.yaml` |
| **Crate** | `agt-provider` |

## 作用

Provider 告诉 agt 把模型请求发往哪里。OpenAI、Anthropic、Google Gemini、API 网关以及实现了 OpenAI API 的本地服务（如 Ollama、vLLM）都可以。每个 [Model](./model.md) 选择它的 Provider 使用的接口格式。

## 怎么用

**Web 控制台：** **资源库** → **新建资源** → Provider。填写名称、base URL 和 API Key。

**CLI：**

```bash
agt provider create openai --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY
agt provider list-models openai
agt provider update openai --base-url https://gateway.example.com/v1
```

**Agent 工具：**

```json
{"kind":"provider","reference":"main","create":true,
 "data":{"base_url":"https://api.openai.com/v1","api_key":"sk-..."}}
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `--base-url` | API 根地址，通常以 `/v1` 结尾 |
| `--api-key-env <变量名>` | 命令运行时从环境变量读取密钥 |
| `--api-key-stdin` | 从标准输入读取密钥，避免留在 shell 历史中 |

## 全部参数

### `agt provider create <名称>`

| 选项 | 必填 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `--base-url <url>` | 是 | | API 根地址 |
| `--api-key <密钥>` | 三选一 | | 直接给出密钥（会出现在 shell 历史和进程列表中） |
| `--api-key-stdin` | 三选一 | | 从标准输入读取密钥 |
| `--api-key-env <变量名>` | 三选一 | | 从该环境变量读取密钥 |
| `--type <类型>` | 否 | `openai-compatible` | Provider 类型；目前只有 `openai-compatible` |
| `--category <c>` | 否 | `default` | 分类 |

密钥只在命令运行时读取一次，并保存到 `provider.yaml`。之后修改环境变量不会改变已保存的密钥。

### `agt provider update <名称>`

| 选项 | 含义 |
| --- | --- |
| `--base-url <url>` | 新的 base URL |
| `--api-key <密钥>`、`--api-key-stdin`、`--api-key-env <变量名>` | 新密钥（最多一种） |
| `--category <c>` | 要更新的 Provider 所在分类 |

### 其他命令

| 命令 | 含义 |
| --- | --- |
| `test <引用>` | 校验已保存的配置，不访问服务器 |
| `list-models <引用>` | 用保存的密钥调用 Provider 的 `/models` 接口，不执行推理。依次尝试 OpenAI 兼容、Anthropic 和 Gemini 的认证方式。 |
| `list`、`info`、`delete`、`category` | 见[资源通用操作](./resources.md#通用-cli-命令) |

### 文件：`provider.yaml`

```yaml
name: openai
type: openai-compatible
base_url: https://api.openai.com/v1
api_key: sk-...
```

| 字段 | 含义 |
| --- | --- |
| `name` | 与目录名相同 |
| `type` | `openai-compatible` |
| `base_url` | API 根地址 |
| `api_key` | 保存的密钥，明文 |

### 工具字段（`resource_write`，`kind: "provider"`）

| 字段 | 创建时 | 含义 |
| --- | --- | --- |
| `base_url` | 必填 | API 根地址 |
| `api_key` | 二选一 | 密钥 |
| `api_key_env` | 二选一 | agt 进程的环境变量名；保存其当前值 |

更新时可以提供任意字段。任何工具、列表或错误都不会返回密钥。

### 相关工具

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `provider_models` | `reference` | 用保存的密钥列出 Provider 的模型 |
| `resource_test` | `kind: "provider"`, `reference` | 只校验配置 |

## 相关模块

- [Model](./model.md)：使用此 Provider 的模型
- [资源通用操作](./resources.md)：分类、删除、通用工具
