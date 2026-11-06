import type { AccountSwitchBlock } from './block-types'
import { formatVerbAt } from '../ui/dates'
import { t } from './locales/text'
import { windowNameInSentence } from '../settings/window-name'

/**
 * The neutral line an agent's move to another account reads as
 * (`crates/provider-registry/src/rotation.rs`): "alex@example.com reached
 * its 5-hour limit, resetting at 9:00 PM. Continuing with sam@example.com."
 *
 * The line says why the agent moved. For a limit it names the window in the
 * viewer's language as a sentence names it, and the reset in their time zone,
 * and leaves out each part the block does not know.
 */
export function accountSwitchText(
  block: Pick<AccountSwitchBlock, 'cause' | 'from' | 'to' | 'window' | 'resetsAt'>,
  now: Date,
): string {
  const { from, to } = block
  if (block.cause === 'signed_out') {
    return t('transcript.accountSwitch.signedOut', { from, to })
  }
  if (block.cause === 'refused') {
    return t('transcript.accountSwitch.refused', { from, to })
  }
  const window = block.window ? windowNameInSentence(block.window) : null
  const reset = block.resetsAt ? formatVerbAt('resetting', block.resetsAt, now) : null
  if (window) {
    return reset === null
      ? t('transcript.accountSwitch.window', { from, to, window })
      : t('transcript.accountSwitch.windowReset', { from, to, window, reset })
  }
  return reset === null
    ? t('transcript.accountSwitch.usage', { from, to })
    : t('transcript.accountSwitch.usageReset', { from, to, reset })
}
