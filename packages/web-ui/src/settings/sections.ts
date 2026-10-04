import {
  Archive,
  Bell,
  Blocks,
  CircleUser,
  Database,
  Keyboard,
  Monitor,
  Plug,
  Settings2,
  Sparkles
} from '@lucide/vue'
import { IN_DEVELOPMENT } from '../ui/disabled'
import type { SettingsNavGroup } from './types'

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
        label: 'Models & Providers',
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
        id: 'plugins',
        label: 'Plugins',
        icon: Blocks,
        keywords: ['plugin', 'extension']
      },
      {
        id: 'mcp',
        label: 'MCP Servers',
        icon: Plug,
        keywords: [
          'tools',
          'transport',
          'stdio',
          'server'
        ],
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
        label: 'Data & Privacy',
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
