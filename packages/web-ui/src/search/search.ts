import { computed, onScopeDispose, ref, watch } from 'vue'

/**
 * A conversation the search window lists: one a query found, or a recent one
 * while the field is empty.
 */
export interface SearchRow {
  conversationId: string
  title: string
  archived: boolean
  /** When the conversation's latest message was written. */
  lastActiveAt: string
  /**
   * The message that matched: its block, the line around the match, and the
   * matched pieces as `[start, end]` UTF-16 offsets in that line, end
   * exclusive. Null for a title match and for a recent conversation.
   */
  match: {
    blockId: string
    text: string
    ranges: readonly (readonly number[])[]
  } | null
}

/** Where the window's results come from: the backend's search, or a specimen's fixtures. */
export type SearchSource = (query: string, signal: AbortSignal) => Promise<SearchRow[]>

/** How long typing pauses before the field's text is searched. */
export const SEARCH_PAUSE_MS = 200

/** The most characters a query has, as the backend bounds it. */
export const SEARCH_QUERY_MAX = 256

/** What the window shows under its field. */
export type SearchView =
  | { kind: 'recent'; rows: SearchRow[] }
  | { kind: 'searching'; rows: SearchRow[] }
  | { kind: 'results'; query: string; rows: SearchRow[] }
  | { kind: 'failed'; query: string; detail: string }

/**
 * The search window's query and what it shows: the recent conversations
 * while the field is empty, and the results of what it holds once typing
 * pauses. A newer query abandons the one in flight, so a late answer never
 * replaces the results of what the field holds now; while one runs, the
 * rows shown last stay.
 */
export function useConversationSearch(source: () => SearchSource, recent: () => SearchRow[]) {
  const query = ref('')
  const answered = ref<Exclude<SearchView, { kind: 'recent' }> | null>(null)
  let timer: ReturnType<typeof setTimeout> | undefined
  let inFlight: AbortController | null = null

  function cancel(): void {
    clearTimeout(timer)
    inFlight?.abort()
    inFlight = null
  }

  async function run(text: string): Promise<void> {
    const controller = new AbortController()
    inFlight = controller
    try {
      const rows = await source()(text, controller.signal)
      if (!controller.signal.aborted) {
        answered.value = { kind: 'results', query: text, rows }
      }
    } catch (error) {
      if (!controller.signal.aborted) {
        answered.value = {
          kind: 'failed',
          query: text,
          detail: error instanceof Error ? error.message : String(error),
        }
      }
    } finally {
      if (inFlight === controller) {
        inFlight = null
      }
    }
  }

  function schedule(text: string, wait: number): void {
    cancel()
    const shown = answered.value?.kind === 'results' || answered.value?.kind === 'searching'
      ? answered.value.rows
      : []
    answered.value = { kind: 'searching', rows: shown }
    timer = setTimeout(() => void run(text), wait)
  }

  watch(query, (typed) => {
    const text = typed.trim()
    if (!text) {
      cancel()
      answered.value = null
      return
    }
    if (answered.value?.kind === 'results' && answered.value.query === text) {
      cancel()
      return
    }
    schedule(text, SEARCH_PAUSE_MS)
  })
  onScopeDispose(cancel)

  const view = computed<SearchView>(() => answered.value ?? { kind: 'recent', rows: recent() })

  return {
    query,
    view,
    /** Asks again for what the field holds, as Retry does after a failure. */
    retry: () => {
      const text = query.value.trim()
      if (text) {
        schedule(text, 0)
      }
    },
    /** Empties the field, as the window does each time it opens. */
    clear: () => {
      query.value = ''
    },
  }
}
