// `demi.message(text, { wait, timeout })`: writes a message in the open
// conversation's composer, or in a new conversation's, in place of any
// draft, sends it with Enter, and waits for the turn it starts to end: the
// conversation no longer runs and has output newer than before the message.
// Background jobs the turn started are not waited for. Answers the
// conversation and how its turn ended. A message the page marks not
// delivered, or a turn that fails, fails the call at once with what the
// page says (browse.md § Failures: a step's failure is never silent).
import type { Locator, Page } from 'playwright'
import type { Slot } from '../slot'
import { Failure, type Tool } from '../tool'
import { conversationId, summary, TURN_MS, waitTurnEnd, type Summary } from './turn'

/** How long a new conversation may take to get its address once its first message is sent. */
const CREATE_MS = 30_000

/** How long the page may take to show a failure the backend already reports. */
const SHOW_MS = 10_000

/** The label of the notice a message the page could not deliver shows (`PendingSubmission.vue`). */
const UNDELIVERED = 'This message was not delivered.'

export interface MessageOptions {
  /** Whether to wait for the turn to end; true unless false. */
  wait?: boolean
  /** How long the turn may take, in milliseconds; ten minutes unless given. */
  timeout?: number
}

export async function message(tool: Tool, text: string, options: MessageOptions = {}): Promise<MessageEnd> {
  return messageOn(tool.slot, await tool.browser.page(), text, options)
}

/** The conversation a message went to, and how its turn ended; null when the call did not wait. */
export type MessageEnd = { conversation: string, turn: Summary | null }

/** `demi.message` on `page`, which shows the web app of `slot`. */
export async function messageOn(slot: Slot, page: Page, text: string, options: MessageOptions = {}): Promise<MessageEnd> {
  const known = conversationId(page)
  const before = known === null ? 0 : (await summary(slot, page, known))?.revision ?? 0
  const composer = page.getByRole('textbox', { name: 'Message', exact: true })
  // The message replaces a draft the composer holds, as the message sent is the one given.
  await composer.fill(text)
  await composer.press('Enter')
  await page.waitForURL((url) => /\/chat\/[^/?#]+/.test(url.pathname), { timeout: CREATE_MS })
  const id = conversationId(page)
  if (id === null) {
    throw new Failure(`The page left the conversation: ${page.url()}`)
  }
  if (options.wait === false) {
    return { conversation: id, turn: null }
  }
  const undelivered = page.getByRole('alert').filter({ hasText: UNDELIVERED })
  const end = await waitTurnEnd(slot, page, id, {
    after: before,
    timeoutMs: options.timeout ?? TURN_MS,
    check: async () => {
      if (await undelivered.isVisible()) {
        throw new Failure(`Conversation ${id}: ${await noticeText(undelivered)}`)
      }
    },
  })
  if (end.outcome === 'failed') {
    // The transcript's last notice is the failed turn's record.
    const notice = page.getByRole('alert').last()
    const shown = await notice.waitFor({ timeout: SHOW_MS }).then(() => true, () => false)
    throw new Failure(shown
      ? `The turn of the message failed in conversation ${id}: ${await noticeText(notice)}`
      : `The turn of the message failed in conversation ${id}, and the page shows no error for it`)
  }
  return { conversation: id, turn: end }
}

/** A notice's text on one line: its sentence and the reason under it, without its controls' labels. */
async function noticeText(notice: Locator): Promise<string> {
  const lines = await notice.locator('p').allInnerTexts()
  return lines.flatMap((line) => line.split('\n')).map((line) => line.trim()).filter(Boolean).join(' ')
}
