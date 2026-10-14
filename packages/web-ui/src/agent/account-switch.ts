import type { AccountSwitchBlock } from './block-types'
import { formatRetryAt } from '../ui/dates'

/**
 * The neutral line an agent's move to another account reads as
 * (`providers.md` § Account rotation): "alex@example.com reached its 5h
 * session limit, resetting at 9:00 PM; continuing with sam@example.com",
 * the reset in the viewer's time zone, and each part the block does not
 * know left out.
 */
export function accountSwitchText(
  block: Pick<AccountSwitchBlock, 'from' | 'to' | 'window' | 'resetsAt'>,
  now: Date,
): string {
  const limit = block.window ? `its ${block.window} limit` : 'its usage limit'
  const reset = block.resetsAt ? `, ${formatRetryAt('resetting', block.resetsAt, now)}` : ''
  return `${block.from} reached ${limit}${reset}; continuing with ${block.to}`
}
