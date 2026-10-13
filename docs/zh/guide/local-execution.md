# 本机命令

`local_execution` 能力让 Native Agent 在运行 agt 的机器上执行 Shell 命令。它默认关闭，需要打开两个开关。

::: warning 这是宿主机权限
命令以运行 agt 的用户权限执行。工作目录不是沙箱，命令做的任何事都不会回滚。只在你信任的 Agent 和模型上开启，
并且只在可以接受这种风险的 home 和账号中使用。
:::

## 开启

1. 在工作空间中允许：在[运行配置](../reference/configuration.md)中设置 `execution.enabled: true`，
   或在**运行设置** → **本机执行**中打开**允许本机命令**。
2. 交给某个 Agent：编辑 Agent 时在**框架内置工具**中勾选**本机命令**，或：

```bash
agt agent update builder --add-builtin-tool local_execution
```

`resource_management` 不包含这项能力，必须单独授予。

## Agent 获得什么

一个工具 `local_exec`：

```json
{ "command": "cargo test -p agt-core", "directory": "/home/me/project" }
```

- 命令在 `/bin/bash --noprofile --norc -c` 中运行。
- `directory` 可选，默认是 Agent 的工作目录（在绑定中设置）。
- 结果包含退出码、耗时，以及截断到 `execution.output_bytes`（默认 64 KiB）的 stdout 和 stderr。
- 命令运行超过 `execution.timeout_seconds`（默认 300 秒）会被终止。

每次调用也会经过[审批策略](./permissions.md)，所以在人工审批模式下，每条命令运行前你都能看到。

## 限制

- 不支持交互式输入，也不支持在命令结束后继续存活的后台服务。
- 长时间的工作请委派为[任务](./tasks.md)，以便等待和取消。
- 停止讨论或取消执行会终止命令的进程组。如果 agt 本身被强行杀死，命令可能仍在运行；重试前请检查宿主机。
- Codex Agent 不使用这项能力。它们通过 Codex 执行命令，受 Codex 沙箱约束。
- 会话中只显示执行了命令，不显示命令内容和输出。完整细节在执行记录中，供操作者查看。
