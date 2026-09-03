import type { InstanceMode, Role } from '../auth/identity'

/**
 * The provider scope (`product.md` § Instance mode): in shared mode the
 * instance's providers have no owner and admins configure them; in
 * isolated mode every provider belongs to the user who made it.
 */
export function providerOwner(mode: InstanceMode, userId: string): string | null {
  return mode === 'shared' ? null : userId
}

export function canConfigureProviders(mode: InstanceMode, role: Role): boolean {
  return mode === 'isolated' || role !== 'user'
}

/** Whether a stored provider's owner column agrees with the mode the instance runs in. */
export function ownerFitsMode(mode: InstanceMode, ownerUserId: string | null): boolean {
  return (ownerUserId === null) === (mode === 'shared')
}
