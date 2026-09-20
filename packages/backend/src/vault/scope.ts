import type { InstanceMode, Role } from '../auth/identity'

/**
 * Who configures providers (`product.md` § Instance mode): the master on a
 * shared instance, whose entries everyone uses; everyone, for their own, on an
 * isolated one. `ProviderVault.ownerFor` names whose entries a user gets.
 */
export function canConfigureProviders(mode: InstanceMode, role: Role): boolean {
  return mode === 'isolated' || role === 'master'
}
