import { shallowReactive } from 'vue'
import { conversationAttachmentSchema, type ConversationAttachment } from '@demicodes/protocol'
import type { AttachmentLookup, MessageAttachment } from '@demicodes/web-ui/markdown/types'
import type { BlobUrl } from '@demicodes/web-ui/agent/media-source'
import { ApiError, apiRequest, readResponse } from './client'

/**
 * Where a share's page asks about attachments: the routes under `path`, whose
 * `…/attachments/:id` answers one, and where its blobs load from. A
 * conversation's own page asks nothing: its transcript brings the
 * descriptions (`docs/decisions/0029-a-conversation-opens-whole.md`).
 */
export interface AttachmentSource {
  path: string
  blobUrl: BlobUrl
}

/** `attachment` as a message shows it, its bytes from `blobUrl`. */
export function describedAttachment(attachment: ConversationAttachment, blobUrl: BlobUrl): MessageAttachment {
  return {
    name: attachment.name,
    mediaType: attachment.mediaType,
    url: blobUrl(attachment.blob, attachment.mediaType),
    size: attachment.size,
    sha256: attachment.blob,
    width: attachment.width,
    height: attachment.height,
    ...(attachment.source ? { source: attachment.source } : {}),
  }
}

/**
 * The answers about a share's attachments, by the address asked, for the
 * page's lifetime. A shared attachment never changes, so the page asks for
 * each once.
 */
const answers = shallowReactive(new Map<string, AttachmentLookup>())

/** The attachments asked for, answered or not, so a render asks once. */
const asked = new Set<string>()

const LOADING: AttachmentLookup = { state: 'loading' }

/**
 * Attachment `id`, such as `a3`, of `source` as the page knows it now. The
 * first lookup asks the backend; a render that read it renders again when
 * the answer arrives.
 */
export function lookupAttachment(source: AttachmentSource, id: string): AttachmentLookup {
  const path = `${source.path}/attachments/${encodeURIComponent(id)}`
  const answer = answers.get(path)
  if (answer) {
    return answer
  }
  if (!asked.has(path)) {
    asked.add(path)
    void ask(source, path)
  }
  return LOADING
}

async function ask(source: AttachmentSource, path: string): Promise<void> {
  try {
    const response = await apiRequest(path)
    const attachment = await readResponse(response, conversationAttachmentSchema)
    answers.set(path, { state: 'found', attachment: describedAttachment(attachment, source.blobUrl) })
  } catch (error) {
    // A share answers every miss with its own code.
    const missing = error instanceof ApiError && error.code === 'share_not_found'
    answers.set(path, missing ? { state: 'missing' } : { state: 'failed' })
  }
}
