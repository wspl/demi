/** Who may do what to whom in Settings › Users. */
import type { SettingsUser, UserRole } from './types'
import type { SentenceText, TitleText } from '../ui/ui-text'
import { t } from './locales/text'

const RANK: Record<UserRole, number> = { user: 0, admin: 1, master: 2 }

/** A role's name in the page's language. */
export function roleLabel(role: UserRole): TitleText {
  switch (role) {
    case 'master':
      return t('roles.master')
    case 'admin':
      return t('roles.admin')
    case 'user':
      return t('roles.user')
  }
}

/** What a role may do, in the page's language. */
export function roleDescription(role: UserRole): SentenceText {
  switch (role) {
    case 'master':
      return t('roles.masterDescription')
    case 'admin':
      return t('roles.adminDescription')
    case 'user':
      return t('roles.userDescription')
  }
}

/** A role can act only on lower roles. */
export function outranks(role: UserRole, other: UserRole): boolean {
  return RANK[role] > RANK[other]
}

/** The roles an account of `role` may give: those below its own. */
export function assignableRoles(role: UserRole): Array<Exclude<UserRole, 'master'>> {
  return (['admin', 'user'] as const).filter((candidate) => outranks(role, candidate))
}

export type UserAction =
  | 'resetPassword'
  | 'changeRole'
  | 'cloudStorage'
  | 'signInCode'
  | 'signOutEverywhere'
  | 'disable'
  | 'enable'

/**
 * What `viewer` may do to `target`, in menu order: nothing to a role not
 * below theirs; to themselves an administrator only sets Cloud Storage,
 * which decides the server's disk, not the account; only the master changes
 * roles; a disabled account refuses every sign-in and has no session to end,
 * so it offers Enable Account instead of Sign-In Code, Sign Out Everywhere
 * and Disable Account.
 */
export function userActions(viewer: SettingsUser, target: SettingsUser): UserAction[] {
  if (viewer.id === target.id) {
    return viewer.role === 'user' ? [] : ['cloudStorage']
  }
  if (!outranks(viewer.role, target.role)) {
    return []
  }
  return [
    'resetPassword',
    ...(viewer.role === 'master' ? (['changeRole'] as const) : []),
    'cloudStorage',
    ...(target.disabled ? (['enable'] as const) : (['signInCode', 'signOutEverywhere', 'disable'] as const)),
  ]
}

/** The most a Cloud disk may be, in GB. */
export const CLOUD_GB_MAX = 1024

/** The size `text` names in GB, a whole number from 1 to 1024, or null when it names none. */
export function parseCloudGb(text: string): number | null {
  const trimmed = text.trim()
  if (!/^\d+$/.test(trimmed)) {
    return null
  }
  const size = Number(trimmed)
  return size >= 1 && size <= CLOUD_GB_MAX ? size : null
}
