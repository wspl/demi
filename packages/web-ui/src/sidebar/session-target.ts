import type { Operation } from '../ui/operations'

/**
 * Signing this browser out, from the account's menu or from Settings: one
 * operation, which the sidebar's account row shows until the page goes to
 * the sign-in page.
 */
export const THIS_SESSION: Operation = { action: 'signOut', target: 'session:this' }
