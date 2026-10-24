import { errorMessage } from '../infra/errors'
import { computed, onScopeDispose, ref, watch } from 'vue'

/**
 * A conversation the search window lists: one a query found, or a recent one
 * while the field is empty.
 */
export interface SearchRow {
  conversationId: string
  /** Null for the placeholder, which shows as New conversation in the reader's language. */
  title: string | null
  /** The matched pieces of the title as `[start, end]` UTF-16 offsets; empty when the title did not match. */
  titleRanges: readonly (readonly number[])[]
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

/**
 * A message of the shown conversation the window lists while it searches in
 * it: who wrote it and when, the line around the match, and the matched
 * pieces as `[start, end]` UTF-16 offsets in that line, end exclusive.
 */
export interface MessageRow {
  blockId: string
  author: 'user' | 'agent'
  time: string
  text: string
  ranges: readonly (readonly number[])[]
}

/** The messages a search of one conversation found, and whether more did than it lists. */
export interface FoundMessages {
  rows: MessageRow[]
  limited: boolean
}

/** Where the messages of the shown conversation come from: the backend's search of it, or a specimen's fixtures. */
export type MessageSource = (query: string, signal: AbortSignal) => Promise<FoundMessages>

/** What a search found: conversations, or the messages of the shown one. */
export type Found =
  | { kind: 'conversations'; rows: SearchRow[] }
  | { kind: 'messages'; rows: MessageRow[]; limited: boolean }

/** How long typing pauses before the field's text is searched. */
export const SEARCH_PAUSE_MS = 200

/**
 * What the window shows under its field: the recent conversations, or a note
 * to type while it searches in one, until the user types; then what was found.
 */
export type SearchView =
  | { kind: 'recent'; rows: SearchRow[] }
  | { kind: 'typeToSearch' }
  | { kind: 'searching'; found: Found | null }
  | { kind: 'results'; query: string; scope: MessageSource | null; found: Found }
  | { kind: 'failed'; query: string; detail: string }

/** The conversations a view lists: the recent ones, or those found; none while it lists messages. */
export function conversationRows(view: SearchView): SearchRow[] {
  if (view.kind === 'recent')
    return view.rows
  const found = view.kind === 'results' || view.kind === 'searching' ? view.found : null
  return found?.kind === 'conversations' ? found.rows : []
}

/**
 * The search window's query and what it shows: the recent conversations while the
 * field is empty, and the results of what it holds once typing pauses. With
 * `searchIn`, it finds the messages of the shown conversation instead, and
 * says to type while the field is empty.
 *
 * A newer query, or a search in another scope, another conversation's
 * included, abandons the one in flight, so a
 * late answer never replaces the results of what the field holds now. While one
 * runs, the rows shown last stay, when they are of the same kind.
 */
export function useConversationSearch(
  source: () => SearchSource,
  recent: () => SearchRow[],
  searchIn: () => MessageSource | null = () => null,
) {
  const query = ref('')
  const answered = ref<Exclude<SearchView, { kind: 'recent' | 'typeToSearch' }> | null>(null)
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
    const scope = searchIn()
    try {
      const found: Found = scope
        ? { kind: 'messages', ...await scope(text, controller.signal) }
        : { kind: 'conversations', rows: await source()(text, controller.signal) }
      if (!controller.signal.aborted) {
        answered.value = { kind: 'results', query: text, scope, found }
      }
    } catch (error) {
      if (!controller.signal.aborted) {
        answered.value = {
          kind: 'failed',
          query: text,
          detail: errorMessage(error),
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
    const now = answered.value
    const shown = now?.kind === 'results' || now?.kind === 'searching' ? now.found : null
    const kind = searchIn() ? 'messages' : 'conversations'
    answered.value = { kind: 'searching', found: shown?.kind === kind ? shown : null }
    timer = setTimeout(() => void run(text), wait)
  }

  /** The search of the conversation searched in, by its identity: another conversation's is another scope. */
  const scope = computed(() => searchIn())
  watch([query, scope], ([typed, now], [, before]) => {
    const text = typed.trim()
    if (!text) {
      cancel()
      answered.value = null
      return
    }
    const shown = answered.value
    if (shown?.kind === 'results' && shown.query === text && shown.scope === now) {
      cancel()
      return
    }
    // Another scope searches at once: the text was typed already.
    schedule(text, now === before ? SEARCH_PAUSE_MS : 0)
  })
  onScopeDispose(cancel)

  const view = computed<SearchView>(() =>
    answered.value ?? (scope.value ? { kind: 'typeToSearch' } : { kind: 'recent', rows: recent() }))

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
