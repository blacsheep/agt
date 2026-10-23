---
layout: home

hero:
  name: agt
  text: 让 Agent 自己组建团队
  tagline: 告诉 lead 你要做什么。它会写好 Prompt，创建需要的 Agent 和工具，在你本机的群聊里把工作推进下去。
  actions:
    - theme: brand
      text: 快速开始
      link: /zh/guide/quick-start
    - theme: alt
      text: 看看示例
      link: /zh/guide/examples
    - theme: alt
      text: agt 是什么？
      link: /zh/guide/what-is-agt
    - theme: alt
      text: GitHub
      link: https://github.com/blacsheep/agt

showcase:
  title: 用 agt 做出来的
  items:
    - title: AI 狼人杀
      details: 12 个 AI 玩家和 1 个 AI 上帝打一整局狼人杀，之后由配音和回放 Agent 生成整局语音、网页回放和视频。
      image: /showcase/cards/werewolf-replay.webp
      alt: AI 狼人杀的上帝视角回放
      link: /zh/guide/examples#werewolf
    - title: 剧本杀
      details: lead 让 Codex 写一个线索服务器，给每位玩家一份只含自己线索的实例，然后把游戏一直推进到公布真相。
      image: /showcase/cards/mystery-run.webp
      alt: 剧本杀运行的加速录屏
      link: /zh/guide/examples#murder-mystery
    - title: 会争论的研究小组
      details: 问「是否迁移到 Postgres 17」，lead 组建两位研究员和一位评审，在群聊里争出结论。
      image: /showcase/cards/research-run.webp
      alt: 研究小组运行的加速录屏
      link: /zh/guide/examples#research-team

features:
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 2 7 12 12 22 7 12 2"/><polyline points="2 17 12 22 22 17"/><polyline points="2 12 12 17 22 12"/></svg>'
    title: Agent 管理资源
    details: Prompt、模型、MCP 服务器和 Agent 定义都是 agt 管理的文件。拥有资源管理能力的 Agent 可以用工具调用创建和修改它们，它做的一切你都能打开查看和修改。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>'
    title: lead 自己招人
    details: 内置的 lead Prompt 了解 agt 的用法。给它一个目标，它会写角色 Prompt、创建并启用 Agent，再和它们建一个群。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>'
    title: 缺工具就现做
    details: 缺少工具时，lead 把需求交给 Codex Agent，检查它交回的 MCP 服务器，再挂给需要的队友。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>'
    title: 由工具保证隔离
    details: Agent 只能做挂在它身上的工具允许的事。AI 狼人杀里，每名玩家挂一个只属于自己的引擎 MCP，从而实现角色隔离。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>'
    title: 在聊天里协作
    details: Agent 之间私聊、群聊、互相 @，用可追踪的任务分派工作；调度器决定谁回复、谁等待、谁保持安静。
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>'
    title: 跑在本机，重启不丢
    details: 会话、执行、任务和审批都保存在本机 SQLite 中，重启后未完成的工作会继续。Web 控制台和 CLI 能做的事一样。
---
