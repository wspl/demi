import type { UserContentBlock } from '@demicodes/core'
import type { MessageEditContent } from '@demicodes/web-ui/agent/message-editing'
import { apiRequest } from './client'
import { displayedUserContentSchema } from '@demicodes/web-ui/transport/protocol'

/** Restores authenticated transcript media to the agent's inline content contract. */
export async function loadEditContent(
  input: MessageEditContent[],
  signal: AbortSignal,
): Promise<UserContentBlock[]> {
  const content = displayedUserContentSchema.array().parse(input)
  return Promise.all(content.map(async (part): Promise<UserContentBlock> => {
    if (part.type === 'text' || part.type === 'reference' || part.type === 'attachment')
      return part
    if (part.type === 'document') {
      if ('data' in part.source)
        return { type: 'document', source: part.source }
      const data = await loadBlob(part.source.ref, signal)
      return {
        type: 'document',
        source: { data, mediaType: part.source.mediaType, fileName: part.source.fileName },
      }
    }
    if (part.source.type !== 'ref')
      return { type: part.type, source: part.source }
    const data = await loadBlob(part.source.ref, signal)
    return { type: part.type, source: { type: 'binary', data, mediaType: part.source.mediaType } }
  }))
}

async function loadBlob(ref: string, signal: AbortSignal): Promise<Uint8Array> {
  const response = await apiRequest(`/blobs/${encodeURIComponent(ref)}`, { signal })
  const data = new Uint8Array(await response.arrayBuffer())
  signal.throwIfAborted()
  return data
}
