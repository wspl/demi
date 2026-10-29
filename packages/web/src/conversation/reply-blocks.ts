import { computed, type ComputedRef } from 'vue'
import { REPLY_BLOCKS_PER_READ } from '@demicodes/protocol'
import type { ReplyBlockLookup } from '@demicodes/web-ui/markdown/types'
import { pageLog } from '@demicodes/web-ui/infra/log'
import { errorFields } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import { replyBlocksAnswerSchema } from '../api/generated/web-api'
import { describeReplyBlocks, replyBlockKey } from './activity'
import { useConversations } from './store'

/**
 * The reply blocks each conversation's page asked the backend for, by
 * `tag:id`, so a block is asked for once in the page's life.
 */
const asked = new Map<string, Set<string>>()
/** The reply blocks each conversation's page asks for at its next read. */
const waiting = new Map<string, Map<string, { tag: string, id: string }>>()

/**
 * The facts of the plugins' reply blocks that the messages of the
 * conversation `conversationId` name, as its page gives them to its
 * messages, from those the conversation holds
 * (`Conversation.replyBlockFacts`): what its transcript's frames and pages
 * brought with the messages that name them.
 *
 * A block with none held, which a copy of the conversation the page kept
 * shows without a read, or a tag that came in pieces before the lane knew the
 * text, is asked for once, with every other one its page lacks then; it is
 * missing until its facts come, and so is one whose facts are null. None
 * while no conversation shows.
 */
export function useReplyBlockFacts(
  conversationId: () => string | undefined,
): ComputedRef<((tag: string, id: string) => ReplyBlockLookup) | undefined> {
  const conversations = useConversations()
  return computed(() => {
    const shown = conversationId()
    if (shown === undefined) {
      return undefined
    }
    const conversation = conversations.items.find((item) => item.id === shown)
    return (tag, id) => {
      const facts = conversation?.replyBlockFacts[replyBlockKey(tag, id)]
      if (facts === undefined && conversation) {
        ask(shown, tag, id)
      }
      return facts === undefined || facts === null ? { state: 'missing' } : { state: 'found', facts }
    }
  })
}

/** Asks for the facts of the block `id` of `tag` of `conversation`, once, with every other block asked for in the same task. */
function ask(conversation: string, tag: string, id: string): void {
  const key = replyBlockKey(tag, id)
  const done = asked.get(conversation) ?? new Set<string>()
  if (done.has(key)) {
    return
  }
  done.add(key)
  asked.set(conversation, done)
  const next = waiting.get(conversation)
  if (next) {
    next.set(key, { tag, id })
    return
  }
  waiting.set(conversation, new Map([[key, { tag, id }]]))
  queueMicrotask(() => void read(conversation))
}

/** Reads the facts of the blocks `conversation`'s page waits for, as many reads as the backend's bound needs, and holds them. */
async function read(conversation: string): Promise<void> {
  const blocks = [...(waiting.get(conversation)?.values() ?? [])]
  waiting.delete(conversation)
  for (let start = 0; start < blocks.length; start += REPLY_BLOCKS_PER_READ) {
    await readSome(conversation, blocks.slice(start, start + REPLY_BLOCKS_PER_READ))
  }
}

async function readSome(conversation: string, blocks: { tag: string, id: string }[]): Promise<void> {
  try {
    const response = await apiRequest('/reply-blocks', { method: 'POST', ...jsonBody({ blocks }) })
    const answer = await readResponse(response, replyBlocksAnswerSchema)
    const held = useConversations().items.find((item) => item.id === conversation)
    if (held) {
      describeReplyBlocks(held, answer.replyBlocks)
    }
  } catch (error) {
    // The blocks stay missing; a page opened again asks again.
    pageLog.warn('reply_blocks.read_failed', { blocks: blocks.length, ...errorFields(error) }, conversation)
  }
}
