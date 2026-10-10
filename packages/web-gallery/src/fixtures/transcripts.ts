import { onScopeDispose, shallowReactive } from 'vue'
import type { Block } from '@demicodes/protocol'
import { findRequest, type ConversationTranscripts, type TranscriptRequest } from '@demicodes/web-ui/files/request-changes'
import { wholeHistory } from '@demicodes/web-ui/agent/history'

/**
 * The gallery's conversation holds every specimen's transcript: what a
 * file pill names, the gallery's files service finds in
 * the transcripts specimens register here, as the product's finds it in the
 * conversation's.
 */
const registered = shallowReactive(new Set<() => ConversationTranscripts>())

/** Registers a specimen's transcripts, read when a request is looked up, until the calling scope ends. */
/** A specimen's whole transcripts, the root's and its helpers', each held as one window. */
export interface GalleryTranscripts {
  blocks: readonly Block[]
  subagents: readonly { id: string; blocks: readonly Block[] }[]
}

export function useGalleryTranscripts(transcripts: () => GalleryTranscripts): void {
  const held = (): ConversationTranscripts => {
    const { blocks, subagents } = transcripts()
    return {
      history: wholeHistory([...blocks]),
      subagents: subagents.map((agent) => ({ id: agent.id, history: wholeHistory([...agent.blocks]) })),
    }
  }
  registered.add(held)
  onScopeDispose(() => {
    registered.delete(held)
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
