import { Bell, CircleUser, Code, Database, Gauge, Keyboard, Monitor, Plug, Settings2, Sparkles, WandSparkles } from '@lucide/vue'
import type { SettingsNavGroup } from './types'

/** The product's settings rail: every section, grouped the way the rail reads them. */
export const SETTINGS_SECTIONS: SettingsNavGroup[] = [
  {
    items: [
      { id: 'general', label: 'General', icon: Settings2, keywords: ['language', 'theme', 'tone', 'accent', 'font size'] },
      { id: 'account', label: 'Account', icon: CircleUser, keywords: ['avatar', 'display name', 'email', 'password', 'sign out', 'delete account'] },
      { id: 'notifications', label: 'Notifications', icon: Bell, keywords: ['browser', 'sound'] },
    ],
  },
  {
    label: 'Agent',
    items: [
      { id: 'models', label: 'Models & providers', icon: Sparkles, keywords: ['api key', 'anthropic', 'openai', 'claude code', 'codex', 'base url', 'catalog'] },
      { id: 'mcp', label: 'MCP servers', icon: Plug, keywords: ['tools', 'transport', 'stdio', 'server'] },
      { id: 'skills', label: 'Skills', icon: WandSparkles, keywords: ['skill', 'workflow', 'git', 'SKILL.md'] },
    ],
  },
  {
    label: 'Workspace',
    items: [
      { id: 'devices', label: 'Devices', icon: Monitor, keywords: ['pairing code', 'claim', 'revoke', 'runner', 'machine'] },
      { id: 'keyboard', label: 'Keyboard', icon: Keyboard, keywords: ['shortcut', 'hotkey', 'binding'] },
      { id: 'data', label: 'Data & privacy', icon: Database, keywords: ['transcripts', 'retention', 'share links', 'export', 'usage data', 'delete'] },
      { id: 'usage', label: 'Usage & billing', icon: Gauge, keywords: ['plan', 'credits', 'invoices', 'spend', 'tokens'] },
    ],
  },
  {
    label: 'Advanced',
    items: [{ id: 'developer', label: 'Developer', icon: Code, keywords: ['logs', 'log level', 'download', 'experiments'] }],
  },
]
