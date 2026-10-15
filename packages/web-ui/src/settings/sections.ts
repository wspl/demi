import {
  Archive,
  Bell,
  Blocks,
  Bot,
  CircleUser,
  Database,
  Info,
  KeyRound,
  Keyboard,
  Monitor,
  ScrollText,
  Settings2,
  Sparkles,
  Users
} from '@lucide/vue'
import { inDevelopment } from '../ui/disabled'
import { APP_SHORTCUTS } from './shortcuts'
import type { SettingsNavGroup, SettingsNavItem, UserRole } from './types'

const notYet = () => ({ disabled: true, disabledReason: inDevelopment() }) as const

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
          { label: 'Message text size', keywords: ['font size', 'text size', 'transcript'] },
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
        label: 'Providers',
        icon: Sparkles,
        keywords: [
          'models',
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
        id: 'credentials',
        label: 'Credentials',
        icon: KeyRound,
        keywords: ['token', 'secret', 'sign-in', 'saved login'],
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
        label: 'Subagents',
        icon: Bot,
        keywords: [
          'profile',
          'spawn',
          'delegate',
          'child agent'
        ],
        settings: [
          { label: 'Subagents' },
          { label: 'Maximum subagents', keywords: ['limit', 'live', 'unlimited'] },
          { label: 'Maximum depth', keywords: ['limit', 'level', 'grandchild', 'unlimited'] },
        ],
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
        ...notYet()
      },
    ],
  },
  {
    items: [
      {
        id: 'about',
        label: 'About',
        icon: Info,
        keywords: ['version', 'licenses', 'open source', 'third-party', 'notices'],
        settings: [{ label: 'Version' }, { label: 'Acknowledgements' }],
      },
    ],
  },
]

/** Settings › Users, which only the master and admins have (`product.md` § User system). */
export const USERS_SECTION: SettingsNavItem = {
  id: 'users',
  label: 'Users',
  icon: Users,
  keywords: ['accounts', 'people', 'admin', 'add user', 'invite', 'reset password', 'role', 'disable', 'sign out everywhere'],
}

/** The rail with Users after Devices, for a user whose role manages accounts; the rail unchanged for a user's. */
export function withUsersSection(groups: SettingsNavGroup[], role: UserRole): SettingsNavGroup[] {
  if (role === 'user') {
    return groups
  }
  return groups.map((group) =>
    group.items.some((item) => item.id === 'devices')
      ? {
          ...group,
          items: group.items.flatMap((item) => (item.id === 'devices' ? [item, USERS_SECTION] : [item])),
        }
      : group,
  )
}
