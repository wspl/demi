// The `web` plugin's page (`packages/plugin-sdk/src/index.ts`): its Web Search
// settings section, under Agent in the settings dialog only, with no entry in
// the app's sidebar, and the types of its state, parameters and results,
// generated from the plugin's manifest.
import { Globe } from '@lucide/vue'
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import { t } from './locales/text'
import WebSearchSettings from './WebSearchSettings.vue'

export * from './generated/plugin'

export const webPage = definePage({
  plugin: PLUGIN,
  settings: {
    group: 'agent',
    // The label and keywords are read where the settings list them, so a language chosen later words them too.
    item: {
      id: 'web-search',
      get label() {
        return t('settings.title')
      },
      icon: Globe,
      get keywords() {
        return t('settings.keywords').split(', ')
      },
    },
    component: WebSearchSettings,
  },
})

/** The page the registry imports (`crates/xtask/src/contracts/registry.rs`). */
export default webPage
