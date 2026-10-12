// The `web` plugin's page (`plugin-pages.md`, `web.md` § Settings › Web
// Search): its settings section, under Agent in the settings dialog only,
// with no entry in the app's sidebar, and the types of its
// state, parameters and results, generated from the plugin's manifest.
import { Globe } from '@lucide/vue'
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import WebSearchSettings from './WebSearchSettings.vue'

export * from './generated/plugin'

export const webPage = definePage({
  plugin: PLUGIN,
  settings: {
    group: 'Agent',
    item: {
      id: 'web-search',
      label: 'Web Search',
      icon: Globe,
      keywords: ['web', 'search', 'Exa', 'API key', 'sign in'],
    },
    component: WebSearchSettings,
  },
})

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default webPage
