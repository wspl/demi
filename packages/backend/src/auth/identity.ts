import type { User, Role } from '@demicodes/product-contracts'
export type { User, Role, InstanceMode } from '@demicodes/product-contracts'

/**
 * Hono's per-request variables under the session gate: `c.get('user')` is the
 * caller.
 */
export type AuthEnv = { Variables: { user: User } }

/**
 * Whether `actor` may act on an account of `target` role: a role acts on
 * strictly lower roles only.
 */
export function outranks(actor: Role, target: Role): boolean {
  return RANK[actor] > RANK[target]
}

const RANK: Record<Role, number> = { master: 2, admin: 1, user: 0 }
