import type { ShareService } from '@demicodes/web-ui/agent/sharing'
import { createShare, removeShare, updateShare } from '../api/shares'

/**
 * The share dialog's service for a conversation, over the owner's share
 * routes. A share the conversation holds already has no link in its state,
 * only its token: the link is the backend's public URL followed by
 * `s/<token>`, as the routes answer it.
 */
export function conversationShareService(conversationId: string, publicUrl: string): ShareService {
  return {
    link: (token) => new URL(`s/${encodeURIComponent(token)}`, publicUrl).href,
    create: async (target) => {
      const { token, url } = await createShare(conversationId, target)
      return { token, url }
    },
    update: async (token) => {
      const { url } = await updateShare(conversationId, token)
      return { token, url }
    },
    remove: (token) => removeShare(conversationId, token),
  }
}
