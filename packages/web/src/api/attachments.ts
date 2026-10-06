import { shallowReactive } from 'vue'
import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import { ApiError, apiRequest, readResponse } from './client'
import { conversationAttachmentSchema } from './generated/web-api'
import { blobUrl } from './uploads'

/**
 * The answers about the attachments the agent uploaded (`commands.md`
 * § Attachment commands), by conversation and number, for the page's
 * lifetime: an attachment never changes, so the page asks for each once.
 */
const answers = shallowReactive(new Map<string, AttachmentLookup>())

/** The attachments asked for, answered or not, so a render asks once. */
const asked = new Set<string>()

const LOADING: AttachmentLookup = { state: 'loading' }

/**
 * Attachment `id`, such as `a3`, of conversation `conversationId` as the
 * page knows it now. The first lookup asks the backend; a render that read
 * it renders again when the answer arrives.
 */
export function lookupAttachment(conversationId: string, id: string): AttachmentLookup {
  const key = `${conversationId}/${id}`
  const answer = answers.get(key)
  if (answer) {
    return answer
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
        width: attachment.width,
        height: attachment.height,
      },
    })
  } catch (error) {
    const missing = error instanceof ApiError && error.code === 'not_found'
    answers.set(key, missing ? { state: 'missing' } : { state: 'failed' })
  }
}
