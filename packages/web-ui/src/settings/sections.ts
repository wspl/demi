import {
  Archive,
  Bell,
  CircleUser,
  Database,
  Keyboard,
  Monitor,
  Plug,
  Settings2,
  Sparkles,
  WandSparkles
} from '@lucide/vue'
import { IN_DEVELOPMENT } from '../ui/disabled'
import type { SettingsNavGroup, SettingsTab } from './types'

const deferred = { disabled: true, disabledReason: IN_DEVELOPMENT } as const

/** The product's settings rail: every section, grouped the way the rail reads them. */
export const SETTINGS_SECTIONS: SettingsNavGroup[] = [
  {
    items: [
      {
        id: 'general',
        label: 'General',
        icon: Settings2,
        keywords: [
          'language',
          'theme',
          'tone',
          'accent',
          'font size'
        ]
      },
      {
        id: 'account',
        label: 'Account',
        icon: CircleUser,
        keywords: [
          'avatar',
          'display name',
          'email',
          'password',
          'sign out',
          'delete account'
        ]
      },
      {
        id: 'notifications',
        label: 'Notifications',
        icon: Bell,
        keywords: ['browser', 'sound'],
        ...deferred
      },
    ],
  },
  {
    label: 'Agent',
    items: [
      {
        id: 'models',
        label: 'Models & providers',
        icon: Sparkles,
        keywords: [
          'api key',
          'anthropic',
          'openai',
          'claude code',
          'codex',
          'base url',
          'catalog'
        ]
      },
      {
        id: 'mcp',
        label: 'MCP servers',
        icon: Plug,
        keywords: [
          'tools',
          'transport',
          'stdio',
          'server'
        ],
        ...deferred
      },
      {
        id: 'skills',
        label: 'Skills',
        icon: WandSparkles,
        keywords: ['skill', 'workflow', 'git', 'SKILL.md'],
        ...deferred
      },
    ],
  },
  {
    label: 'Workspace',
    items: [
      {
        id: 'devices',
        label: 'Devices',
        icon: Monitor,
        keywords: [
          'pairing code',
          'claim',
          'revoke',
          'runner',
          'machine'
        ]
      },
      {
        id: 'archived',
        label: 'Archived',
        icon: Archive,
        keywords: ['archive', 'restore', 'conversation']
      },
      {
        id: 'keyboard',
        label: 'Keyboard',
        icon: Keyboard,
        keywords: ['shortcut', 'hotkey', 'binding']
      },
      {
        id: 'data',
        label: 'Data & privacy',
        icon: Database,
        keywords: [
          'transcripts',
          'retention',
          'share links',
          'export',
          'usage data',
          'delete'
        ],
        ...deferred
      },
    ],
  },
]

/** Whether a rail section can be opened. Deferred pages stay listed and disabled. */
export function isSettingsSectionEnabled(
  id: SettingsTab,
  sections: SettingsNavGroup[] = SETTINGS_SECTIONS
): boolean {
  return sections.flatMap((group) => group.items).some(
    (item) => item.id === id && !item.disabled
  )
}

/** First section a host can land on when the current tab is deferred. */
export function firstEnabledSettingsTab(
  sections: SettingsNavGroup[] = SETTINGS_SECTIONS
): SettingsTab {
  for (const group of sections) {
    const item = group.items.find((entry) => !entry.disabled)
    if (item)
      return item.id
  }
  return 'general'
}
