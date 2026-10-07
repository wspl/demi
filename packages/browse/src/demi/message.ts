// `demi.message(text, { wait, timeout })`: writes a message in the open
// conversation's composer, or in a new conversation's, in place of any
// draft, sends it with Enter, and waits for the turn it starts to end: the
// conversation no longer runs and has output newer than before the message.
// Background jobs the turn started are not waited for. Answers the
// conversation and how its turn ended.
import { Failure, type Tool } from '../tool'
import { conversationId, summary, TURN_MS, waitTurnEnd, type Summary } from './turn'

/** How long a new conversation may take to get its address once its first message is sent. */
const CREATE_MS = 30_000

export interface MessageOptions {
  /** Whether to wait for the turn to end; true unless false. */
  wait?: boolean
  /** How long the turn may take, in milliseconds; ten minutes unless given. */
  timeout?: number
}

export async function message(tool: Tool, text: string, options: MessageOptions = {}): Promise<{ conversation: string, turn: Summary | null }> {
  const page = await tool.browser.page()
  const known = conversationId(page)
  const before = known === null ? 0 : (await summary(tool, page, known))?.revision ?? 0
  // The composer's editor, an editable element with no role of its own, which its label names.
  const composer = page.getByLabel('Message', { exact: true })
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
  const end = await waitTurnEnd(tool, page, id, { after: before, timeoutMs: options.timeout ?? TURN_MS })
  return { conversation: id, turn: end }
}
