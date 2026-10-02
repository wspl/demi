// The `skills` plugin's page (`plugin-pages.md`, `skills.md` § The
// page): its settings section, and the types of its state, parameters and
// results, generated from the plugin's manifest.
import { WandSparkles } from '@lucide/vue'
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import SkillsSettings from './SkillsSettings.vue'

export * from './generated/plugin'

export const skillsPage = definePage({
  plugin: PLUGIN,
  settings: {
    group: 'Agent',
    item: {
      id: 'skills',
      label: 'Skills',
      icon: WandSparkles,
      keywords: ['skill', 'workflow', 'git', 'SKILL.md'],
    },
    component: SkillsSettings,
  },
})

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default skillsPage
