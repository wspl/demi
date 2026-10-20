// The `skills` plugin's page (`packages/plugin-sdk/src/index.ts`): its settings section with
// its sidebar entry, and the types of its state, parameters and results,
// generated from the plugin's manifest.
import { WandSparkles } from '@lucide/vue'
import { definePage } from '@demicodes/plugin-sdk'
import { t } from './locales/text'
import { PLUGIN } from './generated/plugin'
import SkillsSettings from './SkillsSettings.vue'

export * from './generated/plugin'

export const skillsPage = definePage({
  plugin: PLUGIN,
  settings: {
    group: 'agent',
    // Read as the rail draws them, so a language chosen meanwhile rewords them.
    item: {
      id: 'skills',
      get label() {
        return t('title')
      },
      icon: WandSparkles,
      get keywords() {
        return t('keywords').split(',')
      },
    },
    component: SkillsSettings,
    sidebar: true,
  },
})

/** The page the registry imports (`crates/xtask/src/contracts/registry.rs`). */
export default skillsPage
