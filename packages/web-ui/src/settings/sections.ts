import {
  Archive,
  Bell,
  Blocks,
  Bot,
  CircleUser,
  Database,
  Keyboard,
  Monitor,
  ScrollText,
  Settings2,
  Sparkles
} from '@lucide/vue'
import { IN_DEVELOPMENT } from '../ui/disabled'
import { APP_SHORTCUTS } from './shortcuts'
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
        keywords: ['appearance', 'look'],
        settings: [
          { label: 'Language' },
          { label: 'Theme', keywords: ['dark mode', 'light mode', 'appearance'] },
          { label: 'Tone' },
          { label: 'Accent', keywords: ['color', 'colour'] },
          { label: 'Transcript text size', keywords: ['font size', 'text size'] },
        ],
      },
      {
        id: 'account',
        label: 'Account',
        icon: CircleUser,
        settings: [
          { label: 'Avatar', keywords: ['picture', 'photo'] },
          { label: 'Display name', keywords: ['nickname', 'name'] },
          { label: 'Email', keywords: ['address', 'e-mail'] },
          { label: 'Password' },
          { label: 'Sign out', keywords: ['log out', 'logout'] },
          { label: 'Delete account' },
        ],
      },
      {
        id: 'notifications',
        label: 'Notifications',
        icon: Bell,
        keywords: ['browser', 'alert'],
        settings: [
          { label: 'Browser notifications', keywords: ['notify'] },
          { label: 'A turn finishes', keywords: ['done', 'complete'] },
          { label: 'A turn fails', keywords: ['error'] },
          { label: 'Demi needs permission', keywords: ['permission request'] },
        ],
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
        id: 'instructions',
        label: 'Instructions',
        icon: ScrollText,
        keywords: ['personal instructions', 'custom instructions', 'AGENTS.md', 'CLAUDE.md', 'rules'],
        settings: [{ label: 'Personal instructions' }],
      },
      {
        id: 'subagents',
        label: 'Subagent',
        icon: Bot,
        keywords: [
          'profile',
          'spawn',
          'delegate',
          'child agent'
        ],
        settings: [{ label: 'Subagents' }],
      },
      {
        id: 'plugins',
        label: 'Plugins',
        icon: Blocks,
        keywords: ['plugin', 'extension']
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
          'add device',
          'pairing code',
          'claim',
          'revoke',
          'runner',
          'machine'
        ],
        settings: [
          { label: 'Your Cloud environment', keywords: ['cloud', 'reset environment'] },
          { label: 'Storage', keywords: ['disk', 'space'] },
        ],
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
        keyboard: true,
        keywords: ['shortcut', 'hotkey', 'binding'],
        settings: [
          ...APP_SHORTCUTS.map((shortcut) => ({ label: shortcut.action })),
          { label: 'Reset all shortcuts' },
        ],
      },
      {
        id: 'data',
        label: 'Data & Privacy',
        icon: Database,
        keywords: [
          'transcripts',
          'usage data',
          'delete'
        ],
        ...deferred
      },
    ],
  },
]
