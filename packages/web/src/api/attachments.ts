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
 * § Attachment commands), by the address asked, for the page's
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
 * summary, and `node`, when given, names the node whose reply asks, which
 * the backend answers only with an attachment that node's replies show
 * uploaded (`file-previews.md` § Attachment blocks). A number above it names an attachment
 * still to come, which is missing until a summary raises `last` to it, so a
 * reply that named `a9` before `a9` was uploaded shows it once it is. The
 * first lookup of any other asks the backend; a render that read it renders
 * again when the answer arrives, or when `last` rises.
 */
export function lookupAttachment(source: AttachmentSource, id: string, last: () => number = () => Infinity, node?: string): AttachmentLookup {
  const path = `${source.path}/attachments/${encodeURIComponent(id)}${node === undefined ? '' : `?node=${encodeURIComponent(node)}`}`
  const answer = answers.get(path)
  if (answer) {
    return answer
  }
  const number = /^a([1-9]\d*)$/.exec(id)?.[1]
  if (number !== undefined && Number(number) > last()) {
    return MISSING
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
    answers.set(path, {
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
    answers.set(path, missing ? { state: 'missing' } : { state: 'failed' })
  }
}
