import { shallowReactive } from 'vue'
import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import type { BlobUrl } from '@demicodes/web-ui/agent/media-source'
import { ApiError, apiRequest, readResponse } from './client'
import { conversationAttachmentSchema } from './generated/web-api'

/**
 * Where a page asks about attachments: the routes under `path`, a
 * conversation's or a share's (`web-api-conversations.md` § Sharing), whose
 * `…/attachments/:id` answers one, and where its blobs load from.
 */
export interface AttachmentSource {
  path: string
  blobUrl: BlobUrl
}

/**
 * The answers about the conversation's attachments (`commands.md`
 * § Attachment commands), by source and number, for the page's
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
 * Attachment `id`, such as `a3`, of `source` as the page knows it now,
 * where `last` reads the number of the conversation's latest attachment
 * from its summary, or allows any number, as for a share, which has no
 * summary. A number above it names an attachment
 * still to come, which is missing until a summary raises `last` to it, so a
 * reply that named `a9` before `a9` was uploaded shows it once it is. The
 * first lookup of any other asks the backend; a render that read it renders
 * again when the answer arrives, or when `last` rises.
 */
export function lookupAttachment(source: AttachmentSource, id: string, last: () => number = () => Infinity): AttachmentLookup {
  const key = `${source.path}/${id}`
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
    void ask(source, id, key)
  }
  return LOADING
}

async function ask(source: AttachmentSource, id: string, key: string): Promise<void> {
  try {
    const response = await apiRequest(`${source.path}/attachments/${encodeURIComponent(id)}`)
    const attachment = await readResponse(response, conversationAttachmentSchema)
    answers.set(key, {
      state: 'found',
      attachment: {
        name: attachment.name,
        mediaType: attachment.mediaType,
        url: source.blobUrl(attachment.blob, attachment.mediaType),
        size: attachment.size,
        sha256: attachment.blob,
        width: attachment.width,
        height: attachment.height,
      },
    })
  } catch (error) {
    // A share answers every miss with its own code (`web-api-conversations.md` § Sharing).
    const missing = error instanceof ApiError && (error.code === 'not_found' || error.code === 'share_not_found')
    answers.set(key, missing ? { state: 'missing' } : { state: 'failed' })
  }
}
