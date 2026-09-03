import type { InstanceMode, Role } from '../auth/identity'

/**
 * The connection scope (`product.md` § Instance mode): in shared mode the
 * instance's connections have no owner and admins configure them; in
 * isolated mode every connection belongs to the user who made it.
 */
export function connectionOwner(mode: InstanceMode, userId: string): string | null {
  return mode === 'shared' ? null : userId
}

export function canConfigureProviders(mode: InstanceMode, role: Role): boolean {
  return mode === 'isolated' || role !== 'user'
}

/** Whether a stored connection's owner column agrees with the mode the instance runs in. */
export function ownerFitsMode(mode: InstanceMode, ownerUserId: string | null): boolean {
  return (ownerUserId === null) === (mode === 'shared')
}
