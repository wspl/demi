import type { BlobUrl } from '@demicodes/web-ui/agent/media-source'
import type { SharedAgent } from '@demicodes/web-ui/share/shared-agents'
import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import { ApiError, apiRequest, apiUrl, readResponse } from '../api/client'
import { sharedConversationSchema, type SharedConversation } from '../api/generated/web-api'
import { createHistoryReads, type HistoryHolder } from '../conversation/history'

/**
 * The public routes of share `token` (`web-api.md` § Sharing), the only
 * ones its page calls: the page's Content Security Policy lets it connect
 * to nothing else.
 */
export function sharePath(token: string): string {
  return `/shares/${encodeURIComponent(token)}`
}

/** Where the blobs a share's snapshot shows load from. */
export function shareBlobUrl(token: string): BlobUrl {
  return (ref, mediaType) => apiUrl(`${sharePath(token)}/blobs/${encodeURIComponent(ref)}?${new URLSearchParams({ type: mediaType })}`)
}

/**
 * What a shared page holds of share `token`'s snapshot, read the way a
 * conversation's history is: the conversation's own transcript, and each
 * subagent's by its number, which the share's routes take as `node`.
 */
export function shareSnapshot(token: string, agents: readonly SharedAgent[]): HistoryHolder {
  return {
    id: token,
    history: EMPTY_TRANSCRIPT,
    failures: {},
    instructions: [],
    summaries: {},
    shownAt: null,
    subagents: agents.map((agent) => ({ id: String(agent.number), history: EMPTY_TRANSCRIPT, failures: {} })),
  }
}

/** The reads of a snapshot's pages and whole blocks; nothing streams to a snapshot, so no length stands. */
export function createShareReads(signal: () => AbortSignal) {
  return createHistoryReads<HistoryHolder>(signal, () => false, (snapshot) => `${sharePath(snapshot.id)}/transcript`)
}

/** What share `token` shows above its snapshot; null when the link names no share. */
export async function readShare(token: string, signal: AbortSignal): Promise<SharedConversation | null> {
  try {
    return await readResponse(await apiRequest(sharePath(token), { signal }), sharedConversationSchema)
  } catch (error) {
    if (error instanceof ApiError && error.code === 'share_not_found') {
      return null
    }
    throw error
  }
}
