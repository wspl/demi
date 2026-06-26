import { defineConfig } from 'vitepress'

// Public documentation site for Demi. Source lives in docs/; internal working docs
// (docs/internal/) and the raw typedoc HTML (docs/api/) are excluded — the API
// reference is wired in separately as markdown.
export default defineConfig({
  title: 'Demi',
  description: 'A TypeScript toolkit for building agents and coding agents.',
  cleanUrls: true,
  srcExclude: ['internal/**', 'api/**'],
  // TODO: tighten to false once every page is migrated and cross-links are final.
  ignoreDeadLinks: true,
  themeConfig: {
    nav: [
      { text: 'Guides', link: '/guides/add-a-provider' },
      { text: 'Reference', link: '/package-boundaries' },
    ],
    sidebar: {
      '/guides/': [
        {
          text: 'Guides',
          items: [
            { text: 'Add a Provider', link: '/guides/add-a-provider' },
            { text: 'Implement a Host', link: '/guides/implement-a-host' },
            { text: 'Embed the UI', link: '/guides/embed-the-ui' },
          ],
        },
      ],
      '/': [
        {
          text: 'Guides',
          items: [
            { text: 'Add a Provider', link: '/guides/add-a-provider' },
            { text: 'Implement a Host', link: '/guides/implement-a-host' },
            { text: 'Embed the UI', link: '/guides/embed-the-ui' },
          ],
        },
        {
          text: 'Reference',
          items: [
            { text: 'Package Boundaries', link: '/package-boundaries' },
            { text: 'Shell & Yield Control', link: '/shell-yield-control-plan' },
            { text: 'Tool Rendering Spec', link: '/tool-rendering-spec' },
          ],
        },
      ],
    },
    outline: { level: [2, 3] },
  },
})
