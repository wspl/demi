// The `skills` plugin's page (`plugins.md` § The page, `skills.md` § The
// page): its settings section, and the types of its state, parameters and
// results, generated from the plugin's Rust types.
import { WandSparkles } from '@lucide/vue'
import type { PluginPage } from '@demicodes/web-ui/plugins/slots'
import SkillsSettings from './SkillsSettings.vue'

export * from './generated/plugin'

export const skillsPage: PluginPage = {
  plugin: 'skills',
  uses: {
    methods: ['add_source', 'update_source', 'remove_source', 'set_enabled', 'set_source_enabled'],
  },
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
}
