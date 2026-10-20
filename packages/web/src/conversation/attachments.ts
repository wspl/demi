import { computed, type ComputedRef } from 'vue'
import type { AttachmentLookup, MessageAttachment } from '@demicodes/web-ui/markdown/types'
import { lookupAttachment } from '../api/attachments'
import { apiRequest } from '../api/client'
import { blobUrl } from '../api/uploads'
import { useProduct } from '../state/product'

/**
 * The attachments that the messages of the conversation `conversationId` names
 * show, as its page gives them to its messages:
 *
 * - Each comes from the attachment route, as a reply of the node that asks
 *   shows it: the conversation's own agent by the conversation's id, so only
 *   one that agent uploaded.
 * - A number above the conversation's latest attachment, which its summary on
 *   the product channel carries, is missing until a summary raises the latest
 *   to it.
 *
 * A message that read one renders again when its answer arrives or the latest
 * rises to it, and with no other summary. None while no conversation shows.
 */
export function useMessageAttachments(
  conversationId: () => string | undefined,
): ComputedRef<((id: string, node: string | null) => AttachmentLookup) | undefined> {
  const product = useProduct()
  const latest = computed(() => {
    const shown = conversationId()
    return product.snapshot?.conversations.find((item) => item.id === shown)?.lastAttachment ?? 0
  })
  return computed(() => {
    const shown = conversationId()
    return shown === undefined
      ? undefined
      : (id, node) => lookupAttachment({ path: `/conversations/${encodeURIComponent(shown)}`, blobUrl }, id, () => latest.value, node ?? shown)
  })
}

/** The conversation's Host as an attachment's opening asks it. */
export interface OpeningHost {
  /** The conversation whose attachment opens. */
  conversation: string
  /** The device it runs on; null while it is not known. */
  device: string | null
  /** Whether the page knows it is online; a stopped Cloud is not. */
  online: boolean
}

/**
 * The Host path of the file attachment `id` was uploaded from, when opening it
 * shows that file (`docs/decisions/0003-attachment-opens-its-file.md`): its Host is
 * the conversation's and online, and the backend, asked once for this opening,
 * finds the file there. The backend asks the Host the conversation runs on
 * then, only while it is the attachment's, and never wakes a stopped Cloud
 * (`GET …/attachments/:id/file`). Any other answer, a failed one included,
 * opens the copy. A Host the page knows offline, a stopped Cloud included, or
 * another Host is not asked.
 */
export async function attachmentOrigin(id: string, { source }: MessageAttachment, host: OpeningHost): Promise<string | null> {
  if (!source || source.host !== host.device || !host.online) {
    return null
  }
  try {
    // An unreachable backend is no reason to wait: the copy opens.
    await apiRequest(`/conversations/${encodeURIComponent(host.conversation)}/attachments/${encodeURIComponent(id)}/file`, { waits: false })
    return source.path
  } catch {
    // The file is not there, the Host is another or stopped, or the backend
    // could not say: the copy opens.
    return null
  }
}
