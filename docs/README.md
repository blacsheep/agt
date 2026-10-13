# 文档导航

最近整理：2026-10-13。详细文档以中文为主；与代码冲突时以代码为准，并在同一变更中修正文档。

## 目录约定

本目录同时是文档网站（VitePress）的源文件，网站发布在 <https://blacsheep.github.io/agt/>。

| 目录 | 放什么 |
| --- | --- |
| `index.md`、`guide/` | 英文首页与面向新用户的英文指南 |
| `zh/guide/` | 中文使用指南：上手、Web 控制台、命令、功能用法 |
| `zh/architecture/` | 中文架构设计：架构图、模块边界、协议与不变量 |
| `.vitepress/` | 网站配置：导航、侧边栏、搜索、Mermaid |

本地预览（需要 Node.js 22+）：

```bash
cd docs
npm install
npm run dev
```

## 使用指南（zh/guide/）

| 文档 | 内容 |
| --- | --- |
| [Web 控制台](zh/guide/web-console.md) | 启动、界面布局、自动回复规则、常见问题与安全边界 |
| [快速上手与验收](zh/guide/getting-started.md) | 自动化验收、手动接入模型、MCP 客户端配置、响应策略、排错与已知限制 |
| [CLI 参考](zh/guide/cli.md) | 资源管理、分类、递归删除、IM、显式 Agent 执行、Harness |
| [Agent 内置工具](zh/guide/builtin-tools.md) | `resource_management` 的权限与工具契约、创建并启用 Agent、Harness 管理工具 |
| [异步任务协作](zh/guide/tasks.md) | `task_collaboration`：委派、等待、排序延后、取消重试、汇报、事件订阅 |
| [运行配置、记忆与本机执行](zh/guide/config-and-memory.md) | `config.yaml`、Memory 工具、`local_execution`、并行工作与资源锁 |

## 架构与设计（zh/architecture/）

先看[架构图与执行流程](zh/architecture/index.md)，其中有整体架构、资源、消息、Run、任务五张流程图。专题设计：

| 文档 | 内容 |
| --- | --- |
| [调度器](zh/architecture/scheduler.md) | 讨论、参与判断、回复准入、额度、恢复、重试 |
| [Native 执行循环](zh/architecture/native-loop.md) | 循环层次、steering/follow-up、工具阶段契约 |
| [上下文管理](zh/architecture/context.md) | token 预算、自动压缩、会话摘要、重置语义 |
| [Harness：外部执行引擎](zh/architecture/harness.md) | Codex 接入、会话与恢复、执行流程、状态与租约 |
| [权限审批](zh/architecture/permissions.md) | Native 权限策略、Harness 原生审批、人工入口、交互观察 |
| [共享资源管理](zh/architecture/resources.md) | 七类资源的统一入口、更新契约、版本检查 |
| [MCP 工具](zh/architecture/mcp-tools.md) | 开发契约、注册探测、schema 校验、Run 内能力刷新 |
| [会话生命周期](zh/architecture/conversations.md) | 隐藏、退群、解散、负责人、已删除身份 |
| [Web 控制台](zh/architecture/web-console.md) | Node 与 Rust 分工、HTTP 工作流、SSE、界面行为、安全边界 |

## 维护约定

1. 修改行为时，在同一变更中更新对应的现行文档；架构图的更新规则见 [架构图导航](zh/architecture/index.md#更新约定)。
2. 现行文档只描述当前行为，不写“本轮验收”“测试 N 项通过”这类按轮次的记录，这些放在提交说明里。
3. 不在现行文档中写数据库版本号；结构版本由代码中的 `Schema` 定义。
4. 方案落地后，把设计要点并入 `zh/architecture/`；方案草稿和实测记录不进入仓库。
5. 详细文档默认用中文；面向新用户的入门内容同时提供英文版（`guide/`）。代码标识、命令和协议字段保持原文。
6. 新增页面后，在 `.vitepress/config.mts` 的侧边栏中登记，并运行 `npm run build` 确认没有断链。
