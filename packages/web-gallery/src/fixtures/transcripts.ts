import { onScopeDispose, shallowReactive } from 'vue'
import { findRequest, type ConversationTranscripts, type TranscriptRequest } from '@demicodes/web-ui/files/request-changes'

/**
 * The gallery's conversation holds every specimen's transcript: what a
 * file pill names, the gallery's files service finds in
 * the transcripts specimens register here, as the product's finds it in the
 * conversation's.
 */
const registered = shallowReactive(new Set<() => ConversationTranscripts>())

/** Registers a specimen's transcripts, read when a request is looked up, until the calling scope ends. */
export function useGalleryTranscripts(transcripts: () => ConversationTranscripts): void {
  registered.add(transcripts)
  onScopeDispose(() => {
    registered.delete(transcripts)
  })
}

/** A request of the gallery's transcripts, as `ConversationFileService.request` finds it. */
export function galleryRequest(node: string | null, request: string): TranscriptRequest | null {
  for (const transcripts of registered) {
    const found = findRequest(transcripts(), node, request)
    if (found) {
      return found
    }
  }
  return null
}
