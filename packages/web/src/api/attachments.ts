import { shallowReactive } from 'vue'
import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import { ApiError, apiRequest, readResponse } from './client'
import { conversationAttachmentSchema } from './generated/web-api'
import { blobUrl } from './uploads'

/**
 * The answers about the attachments the agent uploaded (`commands.md`
 * § Attachment commands), by conversation and number, for the page's
 * lifetime: an attachment never changes, and a number up to the
 * conversation's latest that has none never will have one, so the page asks
 * for each once.
 */
const answers = shallowReactive(new Map<string, AttachmentLookup>())

/** The attachments asked for, answered or not, so a render asks once. */
const asked = new Set<string>()

const LOADING: AttachmentLookup = { state: 'loading' }

const MISSING: AttachmentLookup = { state: 'missing' }

/**
 * Attachment `id`, such as `a3`, of conversation `conversationId` as the
 * page knows it now, where `last` reads the number of the conversation's
 * latest attachment from its summary. A number above it names an attachment
 * still to come, which is missing until a summary raises `last` to it, so a
 * reply that named `a9` before `a9` was uploaded shows it once it is. The
 * first lookup of any other asks the backend; a render that read it renders
 * again when the answer arrives, or when `last` rises.
 */
export function lookupAttachment(conversationId: string, id: string, last: () => number): AttachmentLookup {
  const key = `${conversationId}/${id}`
  const answer = answers.get(key)
  if (answer) {
    return answer
  }
  const number = /^a([1-9]\d*)$/.exec(id)?.[1]
  if (number !== undefined && Number(number) > last()) {
    return MISSING
  }
  if (!asked.has(key)) {
    asked.add(key)
    void ask(conversationId, id, key)
  }
  return LOADING
}

async function ask(conversationId: string, id: string, key: string): Promise<void> {
  try {
    const response = await apiRequest(
      `/conversations/${encodeURIComponent(conversationId)}/attachments/${encodeURIComponent(id)}`,
    )
    const attachment = await readResponse(response, conversationAttachmentSchema)
    answers.set(key, {
      state: 'found',
      attachment: {
        name: attachment.name,
        mediaType: attachment.mediaType,
        url: blobUrl(attachment.blob, attachment.mediaType),
        sha256: attachment.blob,
        width: attachment.width,
        height: attachment.height,
      },
    })
  } catch (error) {
    const missing = error instanceof ApiError && error.code === 'not_found'
    answers.set(key, missing ? { state: 'missing' } : { state: 'failed' })
  }
}
