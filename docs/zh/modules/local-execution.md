# 本机命令

| 概览 | |
| --- | --- |
| **是什么** | Native Agent 在宿主机上执行的 Shell 命令 |
| **Agent 工具** | `local_exec`（需要 `local_execution`） |
| **Web 控制台** | Agent 表单中的**本机命令**；**运行设置** → **本机执行** |
| **设置** | `execution.*` |
| **Crate** | `agt-service`、`agt-runtime` |

## 作用

`local_exec` 在运行 agt 的机器上执行一条 Bash 命令，返回退出码、耗时和输出。它默认关闭：工作空间必须允许，Agent 也必须被授予该能力。

::: warning 宿主机权限
命令以运行 agt 的用户身份执行。工作目录不是沙箱，任何操作都不会回滚。
:::

## 怎么用

```bash
agt config show > config.yaml   # 设置 execution.enabled: true，然后：
agt config apply config.yaml
agt agent update builder --add-builtin-tool local_execution
```

也可以在**运行设置** → **本机执行**中打开**允许本机命令**，并在 Agent 表单中勾选**本机命令**。每次调用仍要经过[审批策略](./permissions.md)。

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `command` | 命令行 |
| `execution.timeout_seconds` | 每条命令的时限 |

## 全部参数

### `local_exec`

| 参数 | 必填 | 含义 |
| --- | --- | --- |
| `command` | 是 | 用 `/bin/bash --noprofile --norc -c` 执行 |
| `directory` | 否 | 工作目录。默认是 Agent 的工作目录（来自其绑定）；相对路径按它解析。 |

结果：退出码、耗时，以及截断到 `execution.output_bytes` 的 stdout 和 stderr。

### 设置（`config.yaml`）

| 键 | 默认值 | 含义 | 生效时机 |
| --- | --- | --- | --- |
| `execution.enabled` | `false` | 允许拥有 `local_execution` 的 Agent 执行命令 | 关闭：下一条命令即被拒绝。开启：新的执行获得该工具。 |
| `execution.timeout_seconds` | `300` | 每条命令的时限，超时终止进程组。`null` 不限（仍可取消）。 | 新命令 |
| `execution.output_bytes` | `65536` | 返回的 stdout 和 stderr 总量；超出部分截断，命令不会因此停止 | 新命令 |

### 限制

- 仅 Native Agent；Harness Agent 使用 Harness 自己的 Shell 和沙箱。
- 不支持交互式输入，不支持在命令结束后继续存活的后台服务。
- 取消或停止讨论会终止进程组。如果 agt 本身被强行杀死，命令可能仍在运行。
- `resource_management` 不包含这项能力，被创建的 Agent 也不会继承。
- 会话中只显示执行了命令；命令和输出在操作者可见的执行记录中。

## 相关模块

- [Agent](./agent.md)：授予 `local_execution` 并设置工作目录
- [权限审批](./permissions.md)：审批每条命令
