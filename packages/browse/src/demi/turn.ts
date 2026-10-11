// `demi.turn({ timeout })`: waits until the open conversation no longer
// runs, as the page and the backend both tell it. The page counts a turn
// from the send of the message that starts it (`product.md` § Following
// the agent's work): its composer offers Stop and its transcript ends in
// Requesting while the message is on its way, before the backend has it.
// The backend counts a conversation running while its tree works without
// the user (`web-api.md` § Sidebar mutations and read state), so this also
// waits for the subagents and background jobs the turn started.
// `demi.message` waits only for the turn its message starts, which the
// root's latest ended turn names once it ends.
import type { Page } from 'playwright'
import { conversationsSchema, type TurnOutcome } from '@demicodes/web/src/api/generated/web-api'
import type { Slot } from '../slot'
import { Failure, webBase, type Tool } from '../tool'

/**
 * The conversation's status and output revision, and its root's latest
 * ended turn: the block that ended it and how (null before the first).
 */
export type Summary = { status: string, revision: number, lastTurn: { id: string, outcome: TurnOutcome } | null }

/** How often the backend is asked about the conversation while a turn runs. */
const POLL_MS = 250

/** How long a turn may take unless the call says otherwise. */
export const TURN_MS = 10 * 60 * 1000

/** The id of the conversation the page shows, from its address `/chat/<id>`. */
export function conversationId(page: Page): string | null {
  return /\/chat\/([^/?#]+)/.exec(new URL(page.url()).pathname)?.[1] ?? null
}

/**
 * The conversation's status and output revision, as the backend reports
 * them to the signed-in page; null while it has none, as for a new
 * conversation, which the backend holds once its first message is sent.
 */
export async function summary(slot: Slot, page: Page, id: string): Promise<Summary | null> {
  const answer = await page.request.get(`${webBase(slot)}/api/conversations`)
  if (!answer.ok()) {
    throw new Failure(`The backend answered ${answer.status()} for the conversations: ${await answer.text()}`)
  }
  const { conversations } = conversationsSchema.parse(await answer.json())
  const conversation = conversations.find((entry) => entry.id === id)
  return conversation
    ? {
        status: conversation.status,
        revision: conversation.revision,
        lastTurn: conversation.lastTurn && { id: conversation.lastTurn.id, outcome: conversation.lastTurn.outcome },
      }
    : null
}

function running(status: string): boolean {
  return status === 'running' || status === 'compacting'
}

/**
 * Asks the backend about the conversation `id` until `ended` holds for its
 * summary, and answers the summary then; `what` names what is waited for
 * when it does not come within `timeoutMs`. `check` runs before each look
 * at the backend: it reads from the page what `ended` needs besides the
 * summary, or throws to end the wait, as when the page shows the turn can
 * never come.
 */
export async function waitSummary(
  slot: Slot,
  page: Page,
  id: string,
  options: { ended: (summary: Summary) => boolean, what: string, timeoutMs: number, check?: () => Promise<void> },
): Promise<Summary> {
  const deadline = Date.now() + options.timeoutMs
  await options.check?.()
  let last = await summary(slot, page, id)
  while (last === null || !options.ended(last)) {
    if (Date.now() >= deadline) {
      const where = last === null ? 'the backend does not have it yet' : `it is ${last.status} at output revision ${last.revision}`
      throw new Failure(`Waited ${options.timeoutMs / 1000} s for ${options.what} in conversation ${id}; ${where}.`)
    }
    // The backend tells the page of a turn's end over its sync socket, which
    // the tool does not read: it asks again in a moment.
    await page.waitForTimeout(POLL_MS)
    await options.check?.()
    last = await summary(slot, page, id)
  }
  return last
}

/** How long a new conversation may take to get its address once its first message is sent. */
const CREATE_MS = 30_000

/** Waits until the page shows a conversation's address, as a new one gets once its first message is sent. */
export async function conversationAddress(page: Page): Promise<void> {
  await page.waitForURL((url) => /\/chat\/[^/?#]+/.test(url.pathname), { timeout: CREATE_MS })
}

/**
 * The transcript's tail row while the conversation waits (`ActivitySlot.vue`):
 * Requesting, with how long once past a second, or Connecting.
 */
const WAITING_ROW = /^(Requesting( for .+)?|Connecting)$/

/**
 * What the page shows of a turn that runs or is about to: the composer's
 * Stop, which stays from the send of the message that starts the turn to
 * its end, or the transcript's Requesting or Connecting row, which also
 * stands for a recovery the backend has not taken up yet; null when it
 * shows neither.
 */
export async function pageActivity(page: Page): Promise<string | null> {
  // The composer's own Stop, not the one of the agent's browser address bar.
  const stop = page.locator('.composer-shell').getByRole('button', { name: 'Stop', exact: true })
  if (await stop.filter({ visible: true }).count() > 0) {
    return 'Stop'
  }
  const row = page.getByText(WAITING_ROW).filter({ visible: true })
  return await row.count() > 0 ? await row.first().innerText() : null
}

/**
 * Waits until the open conversation no longer runs: no message is on its
 * way, and its turn and every subagent and background job it started have
 * ended. Answers its summary then.
 */
export async function turn(tool: Tool, options: { timeout?: number } = {}): Promise<Summary> {
  return turnOn(tool.slot, await tool.browser.page(), options)
}

/** `demi.turn` on `page`, which shows the web app of `slot`. */
export async function turnOn(slot: Slot, page: Page, options: { timeout?: number } = {}): Promise<Summary> {
  let shown = await pageActivity(page)
  if (shown !== null && conversationId(page) === null) {
    // A new conversation's first message is on its way; the address follows.
    await conversationAddress(page)
  }
  const id = conversationId(page)
  if (!id) {
    throw new Failure(`The page shows no conversation: ${page.url()}`)
  }
  try {
    return await waitSummary(slot, page, id, {
      // The page is asked first: a message it has on its way reaches the
      // backend before it leaves the page, so the backend runs by then.
      check: async () => {
        shown = await pageActivity(page)
        // Nothing on its way and nothing at the backend, as when the
        // message was not delivered: no turn will come.
        if (shown === null && await summary(slot, page, id) === null) {
          throw new Failure(`Conversation ${id} has no message yet, so it has no turn`)
        }
      },
      ended: (summary) => shown === null && !running(summary.status),
      what: 'the conversation to stop running',
      timeoutMs: options.timeout ?? TURN_MS,
    })
  } catch (error) {
    throw error instanceof Failure && shown !== null
      ? new Failure(`${error.message.replace(/\.$/, '')}, and the page still shows ${shown}.`)
      : error
  }
}
