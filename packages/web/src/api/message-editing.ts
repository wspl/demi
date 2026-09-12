import type { UserContentBlock } from '@demicodes/core'
import type { MessageEditContent } from '@demicodes/web-ui/agent/message-editing'
import { apiRequest } from './client'
import { displayedUserContentSchema } from './transcript'

/** Restores authenticated transcript media to the agent's inline content contract. */
export async function loadEditContent(
  input: MessageEditContent[],
  signal: AbortSignal,
): Promise<UserContentBlock[]> {
  const content = displayedUserContentSchema.array().parse(input)
  return Promise.all(content.map(async (part): Promise<UserContentBlock> => {
    if (part.type === 'text' || part.type === 'reference' || part.type === 'attachment'
      || !('type' in part.source) || part.source.type !== 'ref') {
      return part as UserContentBlock
    }
    const source = part.source
    const response = await apiRequest(`/blobs/${encodeURIComponent(source.ref)}`, { signal })
    const data = new Uint8Array(await response.arrayBuffer())
    signal.throwIfAborted()
    if (part.type === 'document') {
      if (!source.fileName) {
        throw new Error('The document attachment has no filename')
      }
      return {
        type: 'document',
        source: { data, mediaType: source.mediaType, fileName: source.fileName },
      }
    }
    return {
      type: part.type,
      source: { type: 'binary', data, mediaType: source.mediaType },
    }
  }))
}
