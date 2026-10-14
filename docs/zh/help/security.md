# 安全模型

agt 是**供单个操作者使用的可信本地工具**。它让 Agent 在你的控制下真正动手做事；隔离互不信任的用户不在它的设计范围内。

## Web 控制台

- 只监听 `127.0.0.1`，校验 `Host` 和 `Origin` 请求头，并要求使用启动时打印的链接中的令牌。
- **链接就是密码。** 持有它的人可以管理所有资源、阅读所有会话、批准工具调用。不要分享、发布它，也不要通过代理暴露端口。
  使用远程服务器时，请用 SSH 隧道。
- 控制台以参数数组调用 `agt` CLI，从不经过 shell。

## Agent 能做什么

Agent 只能做它的工具允许的事：

| 能力 | 来源 |
| --- | --- |
| 聊天、阅读自己所在会话的历史、保存笔记 | 始终具备 |
| 使用某个 MCP 服务 | 被挂载了该服务 |
| 管理 home 中的所有资源、创建并启用 Agent | `resource_management` |
| 委派可追踪任务 | `task_collaboration` |
| 以你的用户身份执行 Shell 命令 | `local_execution` 加 `execution.enabled` |
| 编辑项目中的文件 | Codex Harness，在其沙箱范围内 |

新 Agent 不会继承创建者的能力。`resource_management` 是权限最大的能力：拥有它的 Agent 可以修改任何 Agent 的配置，
包括给其他 Agent 增加能力。只把它交给你信任其管理工作空间的 Agent。

此外，每次 Native 工具调用都要经过[审批策略](../guide/permissions.md)，Codex 自己的请求也会进入审批队列。

## 密钥

- Provider 的 API Key 以明文保存在 home 中的 `providers/.../provider.yaml`。agt 不会在列表、工具结果、日志或错误中返回它们，
  但能读取 home 的人就能读到密钥。请限制 home 目录的访问权限。
- 在 Web 控制台中，密钥通过单个子进程的环境变量传给 CLI，不经过命令行参数。
- 执行记录只保存工具参数的结构和安全的错误码，不保存参数值、工具输出或模型推理。完整的工具详情单独保存，只向操作者和发起调用的 Agent 展示。
- agt 拒绝 home 内的符号链接，包括资源类型目录、category 目录和资源目录。即使有人在其中放了指向 home 外部的链接，agt 也不会借此读写 home 以外的文件。
  home 上层路径中的链接（例如 macOS 的 `/var`）不在检查范围内。

## agt 不防护的范围

- **MCP 服务和本机命令以你的用户身份运行。** agt 不对它们做沙箱隔离。恶意的 MCP 服务或被批准的命令可以做你的账号能做的任何事。
- **已发出的请求无法撤回。** 停止讨论只阻止之后的工作，不能撤回已经发给外部服务的请求。
- **Codex 的文件访问**由 Codex 沙箱和操作系统控制，而不是 agt。共享同一工作目录的 Agent 共享其中的文件。
- **提示词注入。** Agent 读取的内容（网页、文件、其他 Agent 的消息）可能试图改变它们的行为。当 Agent 处理不可信内容时，
  请留意它拥有的强大能力和审批设置。

## 报告漏洞

请通过 GitHub 的 [security advisories](https://github.com/blacsheep/agt/security/advisories/new) 私下报告安全问题，不要提交公开 issue。
