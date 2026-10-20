// `demi.message(text, { files, wait, timeout })`: writes a message in the
// open conversation's composer, or in a new conversation's, in place of any
// draft, attaches `files` from this machine as the composer's file picker
// does, sends it with Enter, and waits for the turn it starts to end: the
// root's latest ended turn is another than before the message.
// Subagents and background jobs the turn started are not waited for, though
// the conversation runs while they do; `demi.turn()` waits for them too.
// Answers the conversation and how its turn ended. A message the page marks
// not delivered, or a turn that fails, fails the call at once with what the
// page says (browse.md § Failures: a step's failure is never silent).
// A draft that holds files fails the call before anything is written:
// writing the message in place of the draft would delete their capsules,
// and the message would go without them, as if it had none.
import type { Locator, Page } from 'playwright'
import type { Slot } from '../slot'
import { Failure, type Tool } from '../tool'
import { conversationAddress, conversationId, summary, TURN_MS, waitSummary, type Summary } from './turn'

/** How long the page may take to show a failure the backend already reports. */
const SHOW_MS = 10_000

export interface MessageOptions {
  /** Files on this machine the message carries, attached through the composer's file input. */
  files?: string[]
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
  // The turn that ended last before the message, which the message's turn follows.
  const before = known === null ? null : (await summary(slot, page, known))?.lastTurn?.id ?? null
  // The composer's field, by the hook `MessageEditor` gives it, in any language.
  const composer = page.getByRole('textbox').and(page.locator('[data-composer]'))
  // A file is a capsule in the message's text.
  const capsules = composer.locator('.message-capsule')
  const held = await capsules.allInnerTexts()
  if (held.length > 0) {
    throw new Failure(
      `The composer's draft holds ${held.map((name) => name.trim()).join(', ')}, which writing the message in its place would delete; `
      + 'remove the files first, or attach them with demi.message(text, { files })',
    )
  }
  // The message replaces a draft the composer holds, as the message sent is the one given.
  await composer.fill(text)
  const files = options.files ?? []
  if (files.length > 0) {
    // The composer's file input, which Attach Files… opens, is the nearest one around its text.
    const input = composer.locator('xpath=ancestor::*[.//input[@type="file"]][1]//input[@type="file"]').first()
    await input.setInputFiles(files)
    // Each file the composer takes joins the message as a capsule; one it refuses says why in a toast.
    await capsules.nth(files.length - 1).waitFor().catch(() => {
      throw new Failure(`The composer did not take every file of ${files.join(', ')}; the page says why`)
    })
  }
  await composer.press('Enter')
  await conversationAddress(page)
  const id = conversationId(page)
  if (id === null) {
    throw new Failure(`The page left the conversation: ${page.url()}`)
  }
  if (options.wait === false) {
    return { conversation: id, turn: null }
  }
  // The notice a message the page could not deliver shows (`AgentMessageVirtualBlock`).
  const undelivered = page.locator('[data-delivery-failure]').getByRole('alert')
  const end = await waitSummary(slot, page, id, {
    ended: (summary) => summary.lastTurn !== null && summary.lastTurn.id !== before,
    what: 'the turn of the message to end',
    timeoutMs: options.timeout ?? TURN_MS,
    check: async () => {
      if (await undelivered.isVisible()) {
        throw new Failure(`Conversation ${id}: ${await noticeText(undelivered)}`)
      }
    },
  })
  if (end.lastTurn?.outcome === 'failed') {
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
