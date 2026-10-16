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
import { t as uiText } from '../locales/text'
import { t as cloudText } from '../cloud/locales/text'
import { t } from './locales/text'
import { APP_SHORTCUTS, shortcutAction } from './shortcuts'
import type { SettingsGroupId, SettingsNavGroup, SettingsNavItem, UserRole } from './types'

/** The words a section or a setting is found by, which the catalogue keeps as a comma-separated list. */
function words(list: string): string[] {
  return list.split(',').map((word) => word.trim())
}


/**
 * The product's settings rail: every section, grouped the way the rail
 * reads them, in the page's language. A function, so a language chosen
 * while the dialog shows rewords the rail and the words it is filtered by.
 */
export function settingsSections(): SettingsNavGroup[] {
  const deferred = { disabled: true, disabledReason: inDevelopment() } as const
  return [
    {
      items: [
        {
          id: 'general',
          label: uiText('settings.general.title'),
          icon: Settings2,
          keywords: words(t('sections.general.keywords')),
          settings: [
            { label: uiText('settings.general.language') },
            { label: uiText('settings.general.theme'), keywords: words(t('sections.general.themeKeywords')) },
            { label: uiText('settings.general.tone') },
            { label: uiText('settings.general.accent'), keywords: words(t('sections.general.accentKeywords')) },
            { label: uiText('settings.general.textSize'), keywords: words(t('sections.general.textSizeKeywords')) },
          ],
        },
        {
          id: 'account',
          label: t('account.title'),
          icon: CircleUser,
          settings: [
            { label: t('account.profile.avatar'), keywords: words(t('sections.account.avatarKeywords')) },
            { label: t('account.profile.displayName'), keywords: words(t('sections.account.displayNameKeywords')) },
            { label: t('account.profile.email'), keywords: words(t('sections.account.emailKeywords')) },
            { label: t('account.profile.password') },
            { label: t('account.session.signOut'), keywords: words(t('sections.account.signOutKeywords')) },
            { label: t('account.danger.delete') },
          ],
        },
        {
          id: 'notifications',
          label: t('notifications.title'),
          icon: Bell,
          keywords: words(t('sections.notifications.keywords')),
          settings: [
            { label: t('notifications.browser.label'), keywords: words(t('sections.notifications.browserKeywords')) },
            { label: t('notifications.when.turnFinishes'), keywords: words(t('sections.notifications.finishesKeywords')) },
            { label: t('notifications.when.turnFails'), keywords: words(t('sections.notifications.failsKeywords')) },
            { label: t('notifications.when.needsPermission'), keywords: words(t('sections.notifications.permissionKeywords')) },
          ],
        },
      ],
    },
    {
      id: 'agent',
      items: [
        {
          id: 'models',
          label: t('providers.title'),
          icon: Sparkles,
          keywords: words(t('sections.providers.keywords')),
        },
        {
          id: 'credentials',
          label: t('credentials.title'),
          icon: KeyRound,
          keywords: words(t('sections.credentials.keywords')),
        },
        {
          id: 'instructions',
          label: t('instructions.title'),
          icon: ScrollText,
          keywords: words(t('sections.instructions.keywords')),
          settings: [{ label: t('instructions.personal') }],
        },
        {
          id: 'subagents',
          label: t('subagents.title'),
          icon: Bot,
          keywords: words(t('sections.subagents.keywords')),
          settings: [
            { label: t('subagents.title') },
            { label: t('subagents.maxLive'), keywords: words(t('sections.subagents.maxLiveKeywords')) },
            { label: t('subagents.maxDepth'), keywords: words(t('sections.subagents.maxDepthKeywords')) },
          ],
        },
        {
          id: 'plugins',
          label: t('plugins.title'),
          icon: Blocks,
          keywords: words(t('sections.plugins.keywords')),
        },
      ],
    },
    {
      id: 'workspace',
      items: [
        {
          id: 'devices',
          label: t('devices.title'),
          icon: Monitor,
          keywords: words(t('sections.devices.keywords')),
          settings: [
            { label: cloudText('environment.title'), keywords: words(t('sections.devices.cloudKeywords')) },
            { label: cloudText('storage.title'), keywords: words(t('sections.devices.storageKeywords')) },
          ],
        },
        {
          id: 'archived',
          label: t('archived.title'),
          icon: Archive,
          keywords: words(t('sections.archived.keywords')),
        },
        {
          id: 'keyboard',
          label: t('keyboard.title'),
          icon: Keyboard,
          keyboard: true,
          keywords: words(t('sections.keyboard.keywords')),
          settings: [
            ...APP_SHORTCUTS.map((shortcut) => ({ label: shortcutAction(shortcut.id) })),
            { label: t('keyboard.resetAll') },
          ],
        },
        {
          id: 'data',
          label: t('data.title'),
          icon: Database,
          keywords: words(t('sections.data.keywords')),
          ...deferred,
        },
      ],
    },
    {
      items: [
        {
          id: 'about',
          label: t('about.title'),
          icon: Info,
          keywords: words(t('sections.about.keywords')),
          settings: [{ label: t('about.version') }, { label: t('about.acknowledgements.title') }],
        },
      ],
    },
  ]
}

/** A rail group's caption in the page's language. */
export function settingsGroupName(id: SettingsGroupId): string {
  return id === 'agent' ? t('sections.agent') : t('sections.workspace')
}

/** Settings › Users, which only the master and admins have (`users.md` § User system). */
export function usersSection(): SettingsNavItem {
  return {
    id: 'users',
    label: t('users.title'),
    icon: Users,
    keywords: words(t('sections.users.keywords')),
  }
}

/** The rail with Users after Devices, for a user whose role manages accounts; the rail unchanged for a user's. */
export function withUsersSection(groups: SettingsNavGroup[], role: UserRole): SettingsNavGroup[] {
  if (role === 'user') {
    return groups
  }
  const users = usersSection()
  return groups.map((group) =>
    group.items.some((item) => item.id === 'devices')
      ? {
          ...group,
          items: group.items.flatMap((item) => (item.id === 'devices' ? [item, users] : [item])),
        }
      : group,
  )
}
