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

const zhGuide = [
  {
    text: '使用指南',
    items: [
      { text: '快速上手与验收', link: '/zh/guide/getting-started' },
      { text: 'Web 控制台', link: '/zh/guide/web-console' },
      { text: 'CLI 参考', link: '/zh/guide/cli' },
      { text: 'Agent 内置工具', link: '/zh/guide/builtin-tools' },
      { text: '异步任务协作', link: '/zh/guide/tasks' },
      { text: '运行配置、记忆与本机执行', link: '/zh/guide/config-and-memory' },
    ],
  },
]

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
      { text: 'MCP 工具', link: '/zh/architecture/mcp-tools' },
      { text: '会话生命周期', link: '/zh/architecture/conversations' },
      { text: 'Web 控制台架构', link: '/zh/architecture/web-console' },
    ],
  },
]

const enGuide = [
  {
    text: 'Introduction',
    items: [
      { text: 'What is agt?', link: '/guide/what-is-agt' },
      { text: 'Getting started', link: '/guide/getting-started' },
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
          { text: 'Guide', link: '/guide/what-is-agt' },
          { text: 'Docs (中文)', link: '/zh/guide/getting-started' },
        ],
        sidebar: { '/guide/': enGuide },
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
          { text: '指南', link: '/zh/guide/getting-started' },
          { text: '架构', link: '/zh/architecture/' },
        ],
        sidebar: { '/zh/guide/': zhGuide, '/zh/architecture/': zhArchitecture },
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
