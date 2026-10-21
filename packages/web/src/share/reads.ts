import { oneRunPerKey } from '@demicodes/utils'
import type { BlobUrl } from '@demicodes/web-ui/agent/media-source'
import { pageBeside, shownWindow } from '@demicodes/web-ui/agent/history'
import type { HistoryReading } from '@demicodes/web-ui/agent/history-reading'
import type { SharedItem } from '@demicodes/web-ui/share/shared-items'
import { addWindow, type HeldTranscript } from '@demicodes/web-ui/transport/protocol'
import { ApiError, apiRequest, apiUrl, readResponse } from '../api/client'
import { createReading } from '../conversation/reading'
import {
  sharedConversationSchema,
  sharedTranscriptPageSchema,
  type SharedConversation,
  type SharedTranscriptPage,
} from '../api/generated/web-api'

/**
 * The public routes of share `token`, the only ones its page calls: the page's
 * Content Security Policy lets it connect to nothing else.
 */
export function sharePath(token: string): string {
  return `/shares/${encodeURIComponent(token)}`
}

/** Where the blobs a share shows load from. */
export function shareBlobUrl(token: string): BlobUrl {
  return (ref, mediaType) => apiUrl(`${sharePath(token)}/blobs/${encodeURIComponent(ref)}?${new URLSearchParams({ type: mediaType })}`)
}

/** The page holds nothing of a share before its first page. */
export const EMPTY_SHARE: HeldTranscript<SharedItem> = { length: 0, windows: [] }

/**
 * A page of a share as the page holds it: its blocks light, and each run of
 * steps not shared in its place under an id of its index, which no block's
 * id takes.
 */
export function sharedPage(page: SharedTranscriptPage): { start: number; length: number; blocks: SharedItem[] } {
  return {
    start: page.start,
    length: page.length,
    blocks: page.blocks.map((entry, offset): SharedItem => entry.type === 'steps_not_shared'
      ? { ...entry, id: `steps-not-shared:${page.start + offset}` }
      : { ...entry, light: true }),
  }
}

/**
 * The reads of share `token`'s pages, from its latest back as the reader
 * scrolls, as a conversation's history reads its pages: `held` is what the page
 * holds, and `hold` takes what a read added. A page one read is fetching already
 * is not fetched twice. Nothing streams to a share, so each page's length stands.
 */
export function createShareReads(
  token: string,
  signal: () => AbortSignal,
  held: () => HeldTranscript<SharedItem>,
  hold: (history: HeldTranscript<SharedItem>) => void,
) {
  const once = oneRunPerKey<string, void>()
  /** The edges of the window a page is being read for now. */
  const reading = createReading()

  function read(query: URLSearchParams): Promise<void> {
    return once(String(query), async () => {
      const response = await apiRequest(`${sharePath(token)}/transcript?${query}`, { signal: signal() })
      hold(addWindow(held(), sharedPage(await readResponse(response, sharedTranscriptPageSchema)), false))
    })
  }

  /** The page beside the window the reader is in, on `side`. */
  function readBeside(side: 'before' | 'after'): Promise<void> {
    const history = held()
    const beside = pageBeside(history, shownWindow(history, null), side)
    if (!beside) {
      return Promise.resolve()
    }
    return reading.track(side, () => read(new URLSearchParams({ [side]: String(beside.index), edge: beside.edge })))
  }

  return {
    /** The edges of the window a page is being read for now. */
    reading: (): HistoryReading => ({ before: reading.has('before'), after: reading.has('after') }),
    readLatest: () => read(new URLSearchParams()),
    readBefore: () => readBeside('before'),
    readAfter: () => readBeside('after'),
  }
}

/** What share `token` shows above what it holds; null when the link names no share. */
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
