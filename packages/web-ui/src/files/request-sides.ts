import { onScopeDispose, shallowRef, watch, type Ref } from 'vue'
import type { EditCopies } from '@demicodes/protocol'
import type { ChangeSides, ReadCallChange } from './changes'
import { FileBrowserError } from './types'

/** What a request's diff shows: none chosen, a read on its way, the sides, or why there are none. */
export type RequestSidesState =
  | { phase: 'idle' }
  /** A side of the selection was not kept. */
  | { phase: 'unavailable' }
  | { phase: 'loading' }
  | { phase: 'ready'; sides: ChangeSides }
  /** A side that is not text. */
  | { phase: 'binary' }
  | { phase: 'failed'; message: string }

export interface RequestSides {
  readonly state: Readonly<Ref<RequestSidesState>>
  /** Reads the selection's sides again now, as Retry does. */
  retry(): void
}

/**
 * The two sides a request's diff shows, read by the blobs `copies` names
 * (`edit-tracking.md` § What the conversation shows). `selection` names
 * what the user chose to see, such as a file's All Changes or one of its
 * edits, and null while nothing is chosen.
 *
 * The transcript the selection comes from is derived anew with every frame
 * of a turn; only another pair of blobs is read. A new pair for the same
 * selection, as All Changes gets once the agent edits the file again,
 * replaces the sides shown when it arrives, without a loading state
 * between; another selection shows the read on its way. Reads end with the
 * calling scope.
 */
export function useRequestSides(
  selection: () => string | null,
  copies: () => EditCopies | null,
  read: () => ReadCallChange | null,
): RequestSides {
  const state = shallowRef<RequestSidesState>({ phase: 'idle' })
  let controller: AbortController | null = null
  /** The selection whose sides `state` holds. */
  let shown: string | null = null

  async function load(): Promise<void> {
    controller?.abort()
    controller = null
    const chosen = selection()
    const reader = read()
    if (chosen === null || !reader) {
      shown = null
      state.value = { phase: 'idle' }
      return
    }
    const pair = copies()
    if (!pair) {
      shown = chosen
      state.value = { phase: 'unavailable' }
      return
    }
    if (chosen !== shown || state.value.phase !== 'ready')
      state.value = { phase: 'loading' }
    shown = chosen
    const current = new AbortController()
    controller = current
    try {
      const sides = await reader(pair, current.signal)
      if (current.signal.aborted)
        return
      state.value = sides === null ? { phase: 'unavailable' } : { phase: 'ready', sides }
    } catch (error) {
      if (current.signal.aborted)
        return
      state.value = error instanceof FileBrowserError && (error.kind === 'binary' || error.kind === 'too-large')
        ? { phase: 'binary' }
        : { phase: 'failed', message: error instanceof Error ? error.message : String(error) }
    }
  }

  // Each source is compared on its own: a frame that names the same
  // selection and blobs reads nothing.
  watch([selection, () => {
    const pair = copies()
    return pair ? `${pair.original}:${pair.modified}` : null
  }, read], () => void load(), { immediate: true })
  onScopeDispose(() => controller?.abort())

  return { state, retry: () => void load() }
}
