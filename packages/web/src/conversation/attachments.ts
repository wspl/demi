import { computed, type ComputedRef } from 'vue'
import type { AttachmentLookup, MessageAttachment } from '@demicodes/web-ui/markdown/types'
import { apiRequest } from '../api/client'
import { describedAttachment } from '../api/attachments'
import { blobUrl } from '../api/uploads'
import { useConversations } from './store'

/**
 * The attachments that the messages of the conversation `conversationId` names
 * show, as its page gives them to its messages, from the descriptions the
 * conversation holds (`Conversation.attachmentDescriptions`): those its
 * transcript's frames and pages brought with the messages that name them.
 * An attachment belongs to the conversation, so a reply of any of its agents
 * shows any of them, whoever uploaded it.
 *
 * A name with no description is missing: one above the conversation's
 * latest attachment may still come, and shows once the socket brings its
 * description as it is uploaded. Nothing is asked of the backend. None while
 * no conversation shows.
 */
export function useMessageAttachments(
  conversationId: () => string | undefined,
): ComputedRef<((id: string) => AttachmentLookup) | undefined> {
  const conversations = useConversations()
  return computed(() => {
    const shown = conversationId()
    const conversation = shown === undefined ? undefined : conversations.items.find((item) => item.id === shown)
    if (shown === undefined) {
      return undefined
    }
    return (id) => {
      const described = conversation?.attachmentDescriptions[id]
      return described ? { state: 'found', attachment: describedAttachment(described, blobUrl) } : { state: 'missing' }
    }
  })
}

/** The conversation's Host as a file card's opening asks it. */
export interface OpeningHost {
  /** The conversation whose card opens. */
  conversation: string
  /** The device it runs on; null while it is not known. */
  device: string | null
  /** Whether the page knows it is online; a stopped Cloud is not. */
  online: boolean
}

/**
 * The path of the file attachment `id` was uploaded from, when a card that
 * opens it shows that file (`docs/decisions/0003-attachment-opens-its-file.md`):
 * its Host is the conversation's and online, and the backend, asked once for
 * this opening, finds the file there. The backend asks the Host the
 * conversation runs on then, only while it is the attachment's, and never
 * wakes a stopped Cloud (`GET …/attachments/:id/file`). Any other answer, a
 * failed one included, opens the copy. A Host the page knows offline, a
 * stopped Cloud included, or another Host is not asked.
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
