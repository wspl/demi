import { computed, type ComputedRef } from 'vue'
import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import { lookupAttachment } from '../api/attachments'
import { blobUrl } from '../api/uploads'
import { useProduct } from '../state/product'

/**
 * The attachments that the messages of the conversation `conversationId`
 * names show, as its page gives them to its messages: each from the attachment
 * route, as a reply of the node that asks shows it, the conversation's own
 * agent by the conversation's id, so only one that agent uploaded; and a number
 * above the conversation's latest attachment, which its summary on the product
 * channel carries, missing until a summary raises the latest to it. A message
 * that read one renders again when its answer arrives or the latest rises to
 * it, and with no other summary. None while no conversation shows.
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
