import { posix } from 'node:path'
import type MarkdownIt from 'markdown-it'
import { withMermaid } from 'vitepress-plugin-mermaid'

const repo = 'https://github.com/blacsheep/agt'

// Docs link to source files with repo-relative paths so they also work when
// browsing on GitHub. On the site those files do not exist, so point them at
// the GitHub source instead.
function sourceLinks(md: MarkdownIt) {
  md.core.ruler.after('inline', 'agt-source-links', (state) => {
    const page: string = state.env.relativePath ?? ''
    for (const block of state.tokens) {
      for (const token of block.children ?? []) {
        if (token.type !== 'link_open') continue
        const href = token.attrGet('href')
        if (!href || /^([a-z]+:|#|\/)/i.test(href)) continue
        const [target, fragment] = href.split('#')
        const resolved = posix.normalize(posix.join('docs', posix.dirname(page), target))
        if (resolved.startsWith('docs/') && !resolved.endsWith('README.md')) continue
        token.attrSet('href', `${repo}/blob/main/${resolved}${fragment ? `#${fragment}` : ''}`)
      }
    }
  })
}

// The English and Chinese user docs mirror each other page for page.
function userDocs(prefix: string, t: Record<string, string>) {
  const item = (key: string, path: string) => ({ text: t[key], link: `${prefix}${path}` })
  return [
    {
      text: t.intro,
      items: [
        item('what', '/guide/what-is-agt'),
        item('install', '/guide/installation'),
        item('quick', '/guide/quick-start'),
        item('concepts', '/guide/concepts'),
      ],
    },
    {
      text: t.guides,
      items: [
        item('models', '/guide/models'),
        item('agents', '/guide/agents'),
        item('conversations', '/guide/conversations'),
        item('lead', '/guide/lead-team'),
        item('mcp', '/guide/mcp'),
        item('codex', '/guide/codex'),
        item('tasks', '/guide/tasks'),
        item('memory', '/guide/memory'),
        item('local', '/guide/local-execution'),
        item('permissions', '/guide/permissions'),
        item('clients', '/guide/mcp-clients'),
      ],
    },
    {
      text: t.modules,
      items: [
        item('mWorkspace', '/modules/workspace'),
        item('mResources', '/modules/resources'),
        item('mProvider', '/modules/provider'),
        item('mModel', '/modules/model'),
        item('mPrompt', '/modules/prompt'),
        item('mSkill', '/modules/skill'),
        item('mMcp', '/modules/mcp'),
        item('mHarness', '/modules/harness'),
        item('mAgent', '/modules/agent'),
        item('mConversations', '/modules/conversations'),
        item('mScheduler', '/modules/scheduler'),
        item('mRuns', '/modules/runs'),
        item('mTasks', '/modules/tasks'),
        item('mMemory', '/modules/memory'),
        item('mPermissions', '/modules/permissions'),
        item('mLocal', '/modules/local-execution'),
        item('mTools', '/modules/tool-service'),
        item('mWeb', '/modules/web-console'),
      ],
    },
    {
      text: t.help,
      items: [item('troubleshooting', '/help/troubleshooting'), item('security', '/help/security')],
    },
  ]
}

const enDocs = userDocs('', {
  intro: 'Introduction',
  what: 'What is agt?',
  install: 'Installation',
  quick: 'Quick start',
  concepts: 'Core concepts',
  guides: 'Guides',
  models: 'Connect models',
  agents: 'Create agents',
  conversations: 'Conversations and auto-reply',
  lead: 'Build a team with a lead',
  mcp: 'MCP tools',
  codex: 'Codex agents',
  tasks: 'Task collaboration',
  memory: 'Memory',
  local: 'Local commands',
  permissions: 'Permissions and approvals',
  clients: 'Use agt from MCP clients',
  modules: 'Modules',
  mWorkspace: 'Workspace and configuration',
  mResources: 'Resources',
  mProvider: 'Provider',
  mModel: 'Model',
  mPrompt: 'Prompt',
  mSkill: 'Skill',
  mMcp: 'MCP',
  mHarness: 'Harness',
  mAgent: 'Agent',
  mConversations: 'Identities and conversations',
  mScheduler: 'Scheduler and discussions',
  mRuns: 'Runs',
  mTasks: 'Tasks',
  mMemory: 'Memory',
  mPermissions: 'Permissions',
  mLocal: 'Local commands',
  mTools: 'Tool service',
  mWeb: 'Web console',
  help: 'Help',
  troubleshooting: 'Troubleshooting',
  security: 'Security model',
})

const zhDocs = userDocs('/zh', {
  intro: '入门',
  what: 'agt 是什么？',
  install: '安装',
  quick: '快速开始',
  concepts: '核心概念',
  guides: '使用指南',
  models: '接入模型',
  agents: '创建 Agent',
  conversations: '对话与自动回复',
  lead: '让 lead 组建团队',
  mcp: 'MCP 工具',
  codex: 'Codex Agent',
  tasks: '任务协作',
  memory: '记忆',
  local: '本机命令',
  permissions: '权限审批',
  clients: '在 MCP 客户端中使用 agt',
  modules: '模块',
  mWorkspace: '工作空间与配置',
  mResources: '资源通用操作',
  mProvider: 'Provider',
  mModel: 'Model',
  mPrompt: 'Prompt',
  mSkill: 'Skill',
  mMcp: 'MCP',
  mHarness: 'Harness',
  mAgent: 'Agent',
  mConversations: '身份与会话',
  mScheduler: '调度器与讨论',
  mRuns: '执行记录',
  mTasks: '任务',
  mMemory: '记忆',
  mPermissions: '权限审批',
  mLocal: '本机命令',
  mTools: '工具服务',
  mWeb: 'Web 控制台',
  help: '帮助',
  troubleshooting: '常见问题与排错',
  security: '安全模型',
})

const zhArchitecture = [
  {
    text: '架构与流程',
    items: [
      { text: '架构图导航', link: '/zh/architecture/' },
      { text: '整体架构', link: '/zh/architecture/overview' },
      { text: '资源管理与能力生效', link: '/zh/architecture/resource-flow' },
      { text: '消息调度流程', link: '/zh/architecture/message-flow' },
      { text: 'Run 执行流程', link: '/zh/architecture/run-flow' },
      { text: '异步任务协作', link: '/zh/architecture/task-flow' },
      { text: '场景：lead 创建 Agent', link: '/zh/architecture/web-lead-create-agent' },
    ],
  },
  {
    text: '专题设计',
    items: [
      { text: '调度器', link: '/zh/architecture/scheduler' },
      { text: 'Native 执行循环', link: '/zh/architecture/native-loop' },
      { text: '上下文管理', link: '/zh/architecture/context' },
      { text: 'Harness：外部执行引擎', link: '/zh/architecture/harness' },
      { text: '权限审批', link: '/zh/architecture/permissions' },
      { text: '共享资源管理', link: '/zh/architecture/resources' },
      { text: '内置工具：权限与契约', link: '/zh/architecture/builtin-tools' },
      { text: '异步任务：设计与语义', link: '/zh/architecture/tasks' },
      { text: '运行配置、记忆与本机执行', link: '/zh/architecture/config-and-memory' },
      { text: 'MCP 工具', link: '/zh/architecture/mcp-tools' },
      { text: '会话生命周期', link: '/zh/architecture/conversations' },
      { text: 'Web 控制台架构', link: '/zh/architecture/web-console' },
    ],
  },
]

export default withMermaid({
  title: 'agt',
  base: '/agt/',
  cleanUrls: true,
  lastUpdated: true,
  srcExclude: ['README.md'],
  head: [['meta', { name: 'theme-color', content: '#3c8772' }]],
  markdown: { config: sourceLinks },
  // The dev server serves mermaid's CommonJS dependencies (such as fastdom)
  // unbundled, which breaks every page. Pre-bundling mermaid converts them.
  vite: { optimizeDeps: { include: ['mermaid'] } },
  themeConfig: {
    socialLinks: [{ icon: 'github', link: repo }],
    search: {
      provider: 'local',
      options: {
        locales: {
          zh: {
            translations: {
              button: { buttonText: '搜索文档', buttonAriaLabel: '搜索文档' },
              modal: {
                noResultsText: '没有找到相关结果',
                resetButtonTitle: '清除查询',
                footer: { selectText: '选择', navigateText: '切换', closeText: '关闭' },
              },
            },
          },
        },
      },
    },
  },
  locales: {
    root: {
      label: 'English',
      lang: 'en-US',
      description: 'Agents that build themselves: prompts, tools, skills and teammates.',
      themeConfig: {
        nav: [
          { text: 'Guide', link: '/guide/what-is-agt', activeMatch: '^/guide/' },
          { text: 'Modules', link: '/modules/workspace', activeMatch: '^/modules/' },
          { text: 'Architecture (中文)', link: '/zh/architecture/' },
        ],
        sidebar: { '/guide/': enDocs, '/modules/': enDocs, '/help/': enDocs },
        editLink: { pattern: `${repo}/edit/main/docs/:path`, text: 'Edit this page on GitHub' },
        footer: { message: 'Released under the MIT License.', copyright: 'Copyright © 2026 blacsheep' },
      },
    },
    zh: {
      label: '简体中文',
      lang: 'zh-CN',
      link: '/zh/',
      description: '让 Agent 拥有资源管理能力：自己写 Prompt、造工具、招队友。',
      themeConfig: {
        nav: [
          { text: '指南', link: '/zh/guide/what-is-agt', activeMatch: '^/zh/guide/' },
          { text: '模块', link: '/zh/modules/workspace', activeMatch: '^/zh/modules/' },
          { text: '架构设计', link: '/zh/architecture/', activeMatch: '^/zh/architecture/' },
        ],
        sidebar: {
          '/zh/guide/': zhDocs,
          '/zh/modules/': zhDocs,
          '/zh/help/': zhDocs,
          '/zh/architecture/': zhArchitecture,
        },
        editLink: { pattern: `${repo}/edit/main/docs/:path`, text: '在 GitHub 上编辑此页' },
        footer: { message: '基于 MIT 许可发布', copyright: 'Copyright © 2026 blacsheep' },
        outline: { label: '本页目录', level: [2, 3] },
        docFooter: { prev: '上一页', next: '下一页' },
        lastUpdated: { text: '最后更新' },
        returnToTopLabel: '回到顶部',
        sidebarMenuLabel: '菜单',
        darkModeSwitchLabel: '主题',
        langMenuLabel: '切换语言',
      },
    },
  },
})
