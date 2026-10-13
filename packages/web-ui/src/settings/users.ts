/** Who may do what to whom in Settings › Users (`product.md` § User system). */
import type { SettingsUser, UserRole } from './types'
import type { TitleText } from '../ui/ui-text'

const RANK: Record<UserRole, number> = { user: 0, admin: 1, master: 2 }

export const ROLE_LABEL: Record<UserRole, TitleText> = {
  master: 'Master',
  admin: 'Admin',
  user: 'User',
}

/** What each role may do, in the words a role is chosen by. */
export const ROLE_DESCRIPTION: Record<UserRole, string> = {
  master: 'Owns the server and manages every account.',
  admin: 'Adds and manages users.',
  user: 'Manages no other account.',
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
  | 'signOutEverywhere'
  | 'disable'
  | 'enable'

/**
 * What `viewer` may do to `target`, in menu order: nothing to themselves or
 * to a role not below theirs; only the master changes roles; a disabled
 * account is already signed out, so it offers Enable Account instead of
 * Sign Out Everywhere and Disable Account.
 */
export function userActions(viewer: SettingsUser, target: SettingsUser): UserAction[] {
  if (viewer.id === target.id || !outranks(viewer.role, target.role)) {
    return []
  }
  return [
    'resetPassword',
    ...(viewer.role === 'master' ? (['changeRole'] as const) : []),
    ...(target.disabled ? (['enable'] as const) : (['signOutEverywhere', 'disable'] as const)),
  ]
}
