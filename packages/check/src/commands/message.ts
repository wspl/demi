// `bun check message <text>`: writes a message in the open conversation's
// composer, or in a new conversation's, sends it with Enter, and waits for
// the turn it starts to end: the conversation no longer runs and has output
// newer than before the message.
import { CheckFailure, parse, timeoutMs, timeoutOption, type Context } from '../command'
import { element } from '../find'
import { conversationId, summary, waitTurnEnd } from '../turn'

const USAGE = 'message <text> [--no-wait] [--timeout <s>]'
const OPTIONS = { ...timeoutOption, 'no-wait': { type: 'boolean' } } as const

/** The composer's editor, whose label is the composer's. */
const COMPOSER = '[contenteditable="true"][aria-label="Message"]'

/** How long a new conversation may take to get its address once its first message is sent. */
const CREATE_MS = 30_000

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length !== 1) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const page = await context.browser.page()
  const known = conversationId(page)
  const before = known === null ? 0 : (await summary(context, page, known))?.revision ?? 0
  const composer = await element(context, COMPOSER)
  await composer.click()
  await page.keyboard.insertText(positionals[0])
  await page.keyboard.press('Enter')
  const began = Date.now()
  await page.waitForURL((url) => /\/chat\/[^/?#]+/.test(url.pathname), { timeout: CREATE_MS })
  const id = conversationId(page)
  if (id === null) {
    throw new CheckFailure(`The page left the conversation: ${page.url()}`)
  }
  if (values['no-wait']) {
    context.print(`Sent in conversation ${id}`)
    return
  }
  const end = await waitTurnEnd(context, page, id, { after: before, timeoutMs: timeoutMs(values.timeout, 600) })
  const took = ((Date.now() - began) / 1000).toFixed(1)
  context.print(`Conversation ${id}: the turn ended after ${took} s, ${end.status}`)
}
