import type { AccountSwitchBlock } from './block-types'
import { formatRetryAt } from '../ui/dates'
import { t } from './locales/text'

/**
 * The neutral line an agent's move to another account reads as
 * (`providers.md` § Account rotation): "alex@example.com reached its 5h
 * session limit, resetting at 9:00 PM. Continuing with sam@example.com.",
 * the reset in the viewer's time zone, and each part the block does not
 * know left out.
 */
export function accountSwitchText(
  block: Pick<AccountSwitchBlock, 'from' | 'to' | 'window' | 'resetsAt'>,
  now: Date,
): string {
  const { from, to, window } = block
  const reset = block.resetsAt ? formatRetryAt('resetting', block.resetsAt, now) : null
  if (window) {
    return reset === null
      ? t('transcript.accountSwitch.window', { from, to, window })
      : t('transcript.accountSwitch.windowReset', { from, to, window, reset })
  }
  return reset === null
    ? t('transcript.accountSwitch.usage', { from, to })
    : t('transcript.accountSwitch.usageReset', { from, to, reset })
}
