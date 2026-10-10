import type { MemoryFileSource } from '@demicodes/web-ui/files/memory-source'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import { galleryAttachment } from './attachments'

/**
 * Where a gallery transcript's messages find the files they name, as a
 * conversation's do: images at the workspace's paths load from `source`,
 * attachments from the gallery's blobs, and a file a click opens goes to
 * `open`.
 */
export function galleryConversationFiles(source: MemoryFileSource, open: NonNullable<ConversationFiles['open']>): ConversationFiles {
  return {
    imageUrl: (path) => source.contents.url(path),
    attachment: galleryAttachment,
    open,
  }
}
