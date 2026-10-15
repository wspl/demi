import type { AccountSwitchBlock } from './block-types'
import { formatRetryAt } from '../ui/dates'
import { t } from './locales/text'
import { windowNameInSentence } from '../settings/window-name'

/**
 * The neutral line an agent's move to another account reads as
 * (`providers.md` § Account rotation): "alex@example.com reached its 5-hour
 * limit, resetting at 9:00 PM. Continuing with sam@example.com.", the
 * window named in the viewer's language as a sentence names it, the reset in
 * their time zone, and each part the block does not know left out.
 */
export function accountSwitchText(
  block: Pick<AccountSwitchBlock, 'from' | 'to' | 'window' | 'resetsAt'>,
  now: Date,
): string {
  const { from, to } = block
  const window = block.window ? windowNameInSentence(block.window) : null
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
