// `demi.turn({ timeout })`: waits until the open conversation no longer
// runs, as the backend reports it: a conversation runs while its tree works
// without the user (`web-api.md` § Sidebar mutations and read state), so
// this waits for its turn and for the helper agents and background jobs the
// turn started. `demi.message` waits only for the turn its message starts,
// which the root's latest ended turn names once it ends.
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
 * at the backend and throws to end the wait, as when the page shows the
 * turn can never come.
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

/**
 * Waits until the open conversation no longer runs: its turn and every
 * helper agent and background job it started have ended. Answers its
 * summary then.
 */
export async function turn(tool: Tool, options: { timeout?: number } = {}): Promise<Summary> {
  const page = await tool.browser.page()
  const id = conversationId(page)
  if (!id) {
    throw new Failure(`The page shows no conversation: ${page.url()}`)
  }
  if (await summary(tool.slot, page, id) === null) {
    throw new Failure(`Conversation ${id} has no message yet, so it has no turn`)
  }
  return waitSummary(tool.slot, page, id, {
    ended: (summary) => !running(summary.status),
    what: 'the conversation to stop running',
    timeoutMs: options.timeout ?? TURN_MS,
  })
}
