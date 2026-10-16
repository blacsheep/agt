# 工作空间与配置

| 概览 | |
| --- | --- |
| **是什么** | 存放一个工作空间的 home 目录，以及它的设置文件 `config.yaml` |
| **CLI** | `agt init`、`agt config`、全局选项 |
| **Web 控制台** | **运行设置** |
| **Crate** | `agt-config` |

## 作用

一个工作空间就是一个 *home* 目录。agt 的所有数据都在里面：资源、会话、执行、任务和设置。不同的 home 完全独立，可以每个项目一个，也可以用临时 home 做实验。

`config.yaml` 保存工作空间设置：调度限制、审批策略、记忆和本机命令。只有操作者能修改它；Agent 即使拥有 `resource_management` 也不能修改。

## 怎么用

**选择 home。** agt 依次使用：命令的 `--home` 选项、`AGT_HOME` 环境变量、`~/.agt`。默认 home 在首次使用时自动创建。

```bash
export AGT_HOME=/data/agt          # 长期生效，写在 ~/.bashrc 或 ~/.zshrc 中
agt --home /tmp/demo agent list    # 只对这条命令生效
agt init ~/projects/game-night     # 显式创建一个新 home
```

**修改设置。** 在 Web 控制台打开**运行设置**，每项设置都显示默认值和生效时机。在 CLI 中：

```bash
agt config show > config.yaml      # 当前设置
# 编辑 config.yaml
agt config apply config.yaml       # 校验并替换整个文件
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `AGT_HOME` | 默认 home 的绝对路径 |
| `--home <目录>` | 单条命令使用的 home |
| `--output json` | 供脚本使用的机器可读输出 |
| `scheduler.replies_per_agent`、`scheduler.discussion_seconds` | Agent 可以持续对话多久；见[调度器](./scheduler.md) |
| `permissions.mode`、`permissions.reviewer_model` | 工具调用如何审批；见[权限审批](./permissions.md) |

## 全部参数

### 全局选项

所有命令都接受。

| 选项 | 默认值 | 含义 |
| --- | --- | --- |
| `--home <目录>` | `$AGT_HOME`，否则 `~/.agt` | 仅本条命令使用的 home |
| `--output <格式>` | `human` | `human`、`json` 或 `jsonl` |
| `--yes` | 关闭 | 跳过破坏性命令的确认。不在终端中运行时必须提供。 |
| `-h`、`--help` | | 任意命令的帮助 |
| `-V`、`--version` | | 打印版本 |

### 环境变量

| 变量 | 含义 |
| --- | --- |
| `AGT_HOME` | 默认 home。必须是绝对路径；空值视为未设置。 |

### `agt init <目录>`

初始化一个 home：创建资源目录、会话数据库、默认 `config.yaml`，以及内置 Prompt `default/default` 和 `default/lead`。其他数据库在首次需要时创建。对已有 home 再次运行会补齐缺少的内容，不覆盖已有文件。

### `agt config`

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `config describe` | | 每项设置的说明、默认值和生效时机 |
| `config show` | | 当前生效的设置 |
| `config apply <文件>` | | 校验 YAML 文件并替换整个配置 |
| `config update` | `--value <json>` `--expected <json>` | 仅当当前配置仍等于 `expected` 时，用 `value` 替换配置（Web 控制台用它避免覆盖并发修改） |

`apply` 会替换整个文件：省略的字段回到默认值。未知字段和非法值会被拒绝。允许“不限”的设置写 `null`；`0` 不被接受。

### `config.yaml`

| 键 | 默认值 | 含义 | 详情 |
| --- | --- | --- | --- |
| `scheduler.concurrency` | `3` | 整个工作空间同时执行的数量 | [调度器](./scheduler.md#全部参数) |
| `scheduler.parallel_work` | `false` | 允许单个 Agent 并行处理任务 | [任务](./tasks.md#全部参数) |
| `scheduler.replies_per_agent` | `100` | 每个 Agent 在一轮讨论中的回复次数 | [调度器](./scheduler.md#全部参数) |
| `scheduler.discussion_seconds` | `1800` | 一轮讨论的授权时长 | [调度器](./scheduler.md#全部参数) |
| `scheduler.model_steps` | `null` | 单次 Native 执行的模型调用次数 | [执行记录](./runs.md#全部参数) |
| `scheduler.timezone` | `null` | Agent 和定时任务使用的时区；`null` 使用本机时区 | [调度器](./scheduler.md#全部参数) |
| `permissions.mode` | `model_review` | Native 工具调用的审批策略 | [权限审批](./permissions.md#全部参数) |
| `permissions.reviewer_model` | `null` | 模型审核使用的模型 | [权限审批](./permissions.md#全部参数) |
| `permissions.review_timeout_seconds` | `30` | 审核模型的时限 | [权限审批](./permissions.md#全部参数) |
| `permissions.request_timeout_seconds` | `600` | 调用等待决定的时限 | [权限审批](./permissions.md#全部参数) |
| `permissions.harness_mode` | `manual` | Codex 审批请求的处理方式 | [权限审批](./permissions.md#全部参数) |
| `memory.enabled` | `true` | 历史搜索与笔记 | [记忆](./memory.md#全部参数) |
| `execution.enabled` | `false` | 允许执行宿主机命令 | [本机命令](./local-execution.md#全部参数) |
| `execution.timeout_seconds` | `300` | 每条命令的时限 | [本机命令](./local-execution.md#全部参数) |
| `execution.output_bytes` | `65536` | 每条命令返回的输出量 | [本机命令](./local-execution.md#全部参数) |

### home 目录结构

```text
<home>/
  config.yaml
  providers/<category>/<name>/provider.yaml
  models/<category>/<name>/model.yaml
  prompts/<category>/<name>/PROMPT.md
  skills/<category>/<package>/SKILL.md
  mcps/<category>/<name>/mcp.json
  harnesses/<category>/<name>/harness.yaml
  agents/<category>/<name>/agent.yaml
  im/im.db               # 身份、会话、消息
  runtime/runtime.db     # 绑定、讨论、执行、任务、审批、Codex 会话
  memory/memory.db       # Agent 的笔记
  .trash/                # 已删除的资源
```

## 相关模块

- [资源通用操作](./resources.md)：资源目录中有什么、如何管理
- [Web 控制台](./web-console.md)：**运行设置**页面
