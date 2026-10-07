// The open conversation's turn, as the backend reports it: a conversation
// runs while its tree works without the user (`web-api.md` § Sidebar
// mutations and read state), and its output revision advances with each
// saved change of output, never with the user's input alone.
import type { Page } from 'playwright'
import { conversationsSchema } from '@demicodes/web/src/api/generated/web-api'
import { CommandFailure, webBase, type Context } from './command'
import { failure } from './find'

export type Summary = { status: string, revision: number }

/** How often the backend is asked about the conversation while a turn runs. */
const POLL_MS = 250

/** The id of the conversation the page shows, from its address `/chat/<id>`. */
export function conversationId(page: Page): string | null {
  return /\/chat\/([^/?#]+)/.exec(new URL(page.url()).pathname)?.[1] ?? null
}

/**
 * The conversation's status and output revision, as the backend reports
 * them to the signed-in page; null while it has none, as for a new
 * conversation, which the backend holds once its first message is sent.
 */
export async function summary(context: Context, page: Page, id: string): Promise<Summary | null> {
  const answer = await page.request.get(`${webBase(context.slot)}/api/conversations`)
  if (!answer.ok()) {
    throw new CommandFailure(`The backend answered ${answer.status()} for the conversations: ${await answer.text()}`)
  }
  const { conversations } = conversationsSchema.parse(await answer.json())
  const conversation = conversations.find((entry) => entry.id === id)
  return conversation ? { status: conversation.status, revision: conversation.revision } : null
}

function running(status: string): boolean {
  return status === 'running' || status === 'compacting'
}

/**
 * Waits until the conversation `id` no longer runs and, when `after` is
 * given, has output newer than that revision: the end of the turn a message
 * started. Answers the conversation's summary then.
 */
export async function waitTurnEnd(
  context: Context,
  page: Page,
  id: string,
  options: { after?: number, timeoutMs: number },
): Promise<Summary> {
  const deadline = Date.now() + options.timeoutMs
  let last = await summary(context, page, id)
  while (last === null || running(last.status) || (options.after !== undefined && last.revision <= options.after)) {
    if (Date.now() >= deadline) {
      const what = options.after === undefined ? 'its turn to end' : 'the turn of the message to end'
      const where = last === null ? 'the backend does not have it yet' : `it is ${last.status} at output revision ${last.revision}`
      throw await failure(context, [`Waited ${options.timeoutMs / 1000} s for ${what} in conversation ${id}; ${where}.`])
    }
    await page.waitForTimeout(POLL_MS)
    last = await summary(context, page, id)
  }
  return last
}
