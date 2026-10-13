---
layout: home

hero:
  name: agt
  text: 别再手搭 Agent，让它们自己搭自己
  tagline: 让 Agent 拥有资源管理能力：按需写 Prompt、造工具、装 Skill、招队友，自己把工作做完。
  actions:
    - theme: brand
      text: 快速上手
      link: /zh/guide/getting-started
    - theme: alt
      text: Web 控制台
      link: /zh/guide/web-console
    - theme: alt
      text: 架构设计
      link: /zh/architecture/

features:
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 2 7 12 12 22 7 12 2"/><polyline points="2 17 12 22 22 17"/><polyline points="2 12 12 17 22 12"/></svg>'
    title: Agent 自己管理资源
    details: Provider、模型、Prompt、Skill、MCP 和 Agent 定义都是资源，Agent 可以通过工具创建和修改，而不只是你手里的配置文件。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>'
    title: lead 自己组建团队
    details: 内置的 lead Prompt 了解整个框架。告诉它目标，它会写角色 Prompt、创建并启用 Agent，再拉进群聊开始协作。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>'
    title: Agent 自己造工具
    details: lead 可以让 Codex 写一个 MCP 服务，像真实运行一样探测验证，再挂载给需要它的队友。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>'
    title: 靠能力隔离，而不是靠提示词
    details: 每个 Agent 能看到什么、能做什么，由挂载的工具决定。比如给剧本杀的每位玩家只挂一个能查看自己剧本的工具，他们就真的看不到别人的剧本。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>'
    title: 团队自己沟通
    details: 私聊、群聊和 @ 提及；可委派任务并持久化汇报；调度器决定谁回复、谁等待、谁保持沉默。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>'
    title: 持久化、跑在本机
    details: 执行、任务、审批和会话都存在本机 SQLite 中，重启后未完成的工作会继续。Web 控制台和 CLI 能力一致。
---
