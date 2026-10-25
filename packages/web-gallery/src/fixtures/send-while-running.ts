import { reactive } from 'vue'
import { DEFAULT_SEND_WHILE_RUNNING, type SendWhileRunning } from '@demicodes/web-ui/agent/send-way'
import type { IdleLifetime } from '@demicodes/web-ui/settings/types'

/**
 * The gallery's stand-in for the user's message preferences:
 *
 * - What Enter does while the agent works.
 * - Whether a turn its offline device stopped resumes by itself once the
 *   device is back.
 * - Whether a turn a restart of Demi stopped resumes by itself once Demi is
 *   back.
 * - Whether a turn a usage limit stopped resumes by itself once the limit
 *   resets.
 * - Whether an idle conversation compacts before its prompt cache expires,
 *   and for which least lifetime.
 *
 * There is one value each for every specimen, so the choice made in the
 * settings specimen is what each composer's Enter does and what each failure
 * record's Resume at checkbox shows, as the product's preference reaches
 * every conversation.
 */
export const galleryMessages = reactive<{
  sendWhileRunning: SendWhileRunning
  resumeAfterRestart: boolean
  resumeAtRetry: boolean
  compactIdle: boolean
  idleLifetime: IdleLifetime
}>({
  sendWhileRunning: DEFAULT_SEND_WHILE_RUNNING,
  resumeAfterRestart: true,
  resumeAtRetry: false,
  compactIdle: false,
  idleLifetime: '1h',
})
