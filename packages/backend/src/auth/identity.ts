/** The three roles (`product.md` § User system): master is the instance's first account; no organizations, no further roles. */
export type Role = 'master' | 'admin' | 'user'

/** The authenticated caller as every route sees it: the user row without its hash. */
export interface User {
  id: string
  username: string
  role: Role
  createdAt: string
}

/** Hono's per-request variables under the session gate: `c.get('user')` is the caller. */
export type AuthEnv = { Variables: { user: User } }

/** Whether `actor` may act on an account of `target` role: a role acts on strictly lower roles only. */
export function outranks(actor: Role, target: Role): boolean {
  return RANK[actor] > RANK[target]
}

const RANK: Record<Role, number> = { master: 2, admin: 1, user: 0 }
