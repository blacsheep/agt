# Docs / 文档

This directory is the source of the documentation site, <https://blacsheep.github.io/agt/>.
本目录是文档网站的源文件。

## Layout / 目录

| Path | Content |
| --- | --- |
| `index.md`, `guide/`, `modules/`, `help/` | English user docs |
| `zh/index.md`, `zh/guide/`, `zh/modules/`, `zh/help/` | 中文用户文档，与英文逐页对应 |
| `zh/architecture/` | 架构与设计文档（仅中文），面向贡献者 |
| `.vitepress/` | Site config: navigation, sidebars, search, Mermaid |

User docs (English / 中文):

| Section | English | 中文 |
| --- | --- | --- |
| Start | [What is agt?](guide/what-is-agt.md) · [Installation](guide/installation.md) · [Quick start](guide/quick-start.md) · [Examples](guide/examples.md) · [Core concepts](guide/concepts.md) | [agt 是什么？](zh/guide/what-is-agt.md) · [安装](zh/guide/installation.md) · [快速开始](zh/guide/quick-start.md) · [示例](zh/guide/examples.md) · [核心概念](zh/guide/concepts.md) |
| Guides | [Models](guide/models.md) · [Agents](guide/agents.md) · [Conversations](guide/conversations.md) · [Lead](guide/lead-team.md) · [MCP](guide/mcp.md) · [Codex](guide/codex.md) · [Tasks](guide/tasks.md) · [Memory](guide/memory.md) · [Local commands](guide/local-execution.md) · [Permissions](guide/permissions.md) · [MCP clients](guide/mcp-clients.md) | [模型](zh/guide/models.md) · [Agent](zh/guide/agents.md) · [对话](zh/guide/conversations.md) · [lead](zh/guide/lead-team.md) · [MCP](zh/guide/mcp.md) · [Codex](zh/guide/codex.md) · [任务](zh/guide/tasks.md) · [记忆](zh/guide/memory.md) · [本机命令](zh/guide/local-execution.md) · [权限](zh/guide/permissions.md) · [MCP 客户端](zh/guide/mcp-clients.md) |
| Modules | [Workspace](modules/workspace.md) · [Resources](modules/resources.md) · [Provider](modules/provider.md) · [Model](modules/model.md) · [Prompt](modules/prompt.md) · [Skill](modules/skill.md) · [MCP](modules/mcp.md) · [Harness](modules/harness.md) · [Agent](modules/agent.md) · [Conversations](modules/conversations.md) · [Scheduler](modules/scheduler.md) · [Runs](modules/runs.md) · [Tasks](modules/tasks.md) · [Memory](modules/memory.md) · [Permissions](modules/permissions.md) · [Local commands](modules/local-execution.md) · [Tool service](modules/tool-service.md) · [Web console](modules/web-console.md) | [工作空间](zh/modules/workspace.md) · [资源](zh/modules/resources.md) · [Provider](zh/modules/provider.md) · [Model](zh/modules/model.md) · [Prompt](zh/modules/prompt.md) · [Skill](zh/modules/skill.md) · [MCP](zh/modules/mcp.md) · [Harness](zh/modules/harness.md) · [Agent](zh/modules/agent.md) · [身份与会话](zh/modules/conversations.md) · [调度器](zh/modules/scheduler.md) · [执行记录](zh/modules/runs.md) · [任务](zh/modules/tasks.md) · [记忆](zh/modules/memory.md) · [权限审批](zh/modules/permissions.md) · [本机命令](zh/modules/local-execution.md) · [工具服务](zh/modules/tool-service.md) · [Web 控制台](zh/modules/web-console.md) |
| Help | [Troubleshooting](help/troubleshooting.md) · [Security](help/security.md) | [排错](zh/help/troubleshooting.md) · [安全](zh/help/security.md) |

Architecture (Chinese): [架构图与执行流程](zh/architecture/index.md).

## Preview / 本地预览

Requires Node.js 22+:

```bash
cd docs
npm install
npm run dev     # http://localhost:5173/agt/
npm run build   # fails on dead links
```

## Conventions / 维护约定

1. **English and Chinese user docs mirror each other.** Every page under `guide/`, `modules/`
   and `help/` has a counterpart under `zh/` with the same file name and the same sections.
   Change both in the same commit. 中英文用户文档逐页对应，修改时同一提交中同步两边。
2. **User docs describe how to use agt**, not how it is implemented. Guides explain tasks step by
   step; module pages list every command, file field, tool parameter and setting of one module. Implementation details,
   invariants and module boundaries go in `zh/architecture/`.
3. When behavior changes, update the affected docs in the same change. Docs describe current
   behavior only; test results and change history belong in commit messages.
4. Keep code identifiers, commands and protocol fields in their original form.
5. Register new pages in `.vitepress/config.mts` and run `npm run build` to check for dead links.
