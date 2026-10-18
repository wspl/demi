import type { StatusDotTone } from '../ui/StatusDot.vue'
import { statusTone } from '../agent/conversation-status'
import type { SidebarConversation } from './types'

/**
 * The tone of a conversation's sidebar dot (`conversations.md` § Conversations
 * and projects), which the row and the browser tab both read: yellow while a
 * permission request waits unread, over every other mark; otherwise the
 * conversation's state, the accent while it runs, green or red until it is
 * read; none for a settled row.
 */
export function conversationDotTone(
  conversation: Pick<SidebarConversation, 'status' | 'needsYou' | 'unread'>,
): StatusDotTone | null {
  if (conversation.needsYou)
    return 'attention'
  return statusTone(conversation.status, { read: !conversation.unread })
}
