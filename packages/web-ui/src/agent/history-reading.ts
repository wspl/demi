import { inject, provide, type InjectionKey } from 'vue'

/**
 * Which edges of a transcript's window the page is reading a page for, as a
 * view shows them: a loading row shows at an edge only while the read lasts.
 */
export interface HistoryReading {
  before: boolean
  after: boolean
}

/**
 * The reads under way for the transcript of the agent `node`, the root's for
 * null. It is read where the list draws, so what it reads is tracked.
 */
export type HistoryReadingLookup = (node: string | null) => HistoryReading

const NONE: HistoryReading = { before: false, after: false }

const historyReadingKey: InjectionKey<HistoryReadingLookup> = Symbol('history-reading')

/** Gives the transcripts below the reads the page has under way. */
export function provideHistoryReading(lookup: HistoryReadingLookup): void {
  provide(historyReadingKey, lookup)
}

/** What the transcripts' host reads; nothing where it gives none, as the gallery's lists. */
export function useHistoryReading(): HistoryReadingLookup {
  return inject(historyReadingKey, () => NONE)
}
