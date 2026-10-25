---
layout: home

hero:
  name: agt
  text: Agents that build their own team
  tagline: Tell a lead agent what you want done. It writes the prompts, creates the agents and the tools they need, and runs the work in a group chat on your machine.
  actions:
    - theme: brand
      text: Get started
      link: /guide/quick-start
    - theme: alt
      text: See examples
      link: /guide/examples
    - theme: alt
      text: What is agt?
      link: /guide/what-is-agt
    - theme: alt
      text: GitHub
      link: https://github.com/blacsheep/agt

showcase:
  title: Built with agt
  items:
    - title: AI Werewolf
      details: Twelve AI players and an AI God play a full game of Werewolf, then voice and replay agents turn it into audio, a web replay and a video.
      image: /showcase/cards/werewolf-replay.webp
      alt: God's-view replay of an AI Werewolf game
      link: /guide/examples#werewolf
    - title: A murder mystery
      details: The lead has Codex write a clue server, gives each player a copy that holds only their own clue, and runs the game to the reveal.
      image: /showcase/cards/mystery-run.webp
      alt: Sped-up recording of the murder-mystery run
      link: /guide/examples#murder-mystery
    - title: A research team that argues
      details: Asked whether to migrate to Postgres 17, the lead hires two researchers and a reviewer who argue it out in a group chat.
      image: /showcase/cards/research-run.webp
      alt: Sped-up recording of the research run
      link: /guide/examples#research-team

features:
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 2 7 12 12 22 7 12 2"/><polyline points="2 17 12 22 22 17"/><polyline points="2 12 12 17 22 12"/></svg>'
    title: Agents manage resources
    details: Prompts, models, MCP servers and agent definitions are files agt manages. An agent with resource management can create and change them with tool calls, and you can read and edit everything it made.
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>'
    title: A lead that hires its team
    details: The built-in lead prompt knows how agt works. Give it a goal and it writes role prompts, creates and enables the agents, and opens a group chat with them.
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>'
    title: Tools built on request
    details: When a tool is missing, the lead hands a spec to a coding agent on Codex, Claude Code or DeepSeek Harness, checks the MCP server it gets back, and attaches it to the teammates that need it.
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>'
    title: Isolation enforced by tools
    details: An agent can only do what its attached tools allow. In AI Werewolf, each player has an engine MCP server of its own, which keeps the roles apart.
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>'
    title: Teams that work in chats
    details: Agents use direct and group chats and @mention each other. They hand off tracked tasks, and a scheduler decides who replies, who waits and who stays quiet.
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/></svg>'
    title: Local, and survives restarts
    details: Conversations, runs, tasks and approvals are kept in SQLite on your machine, and pending work continues after a restart. The Web console and the CLI can do the same things.
---
