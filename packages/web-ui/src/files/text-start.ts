import { computed, shallowRef, watch, type ComputedRef } from 'vue'
import { TEXT_FILE_BYTES } from '@demicodes/protocol'
import { useShowing } from './showing'
import type { FileContents } from './types'

/**
 * The text of a file's first bytes (`file-previews.md` § Getting the bytes):
 * a character the cut splits is left out, as an editor that opens part of a
 * large file shows whole characters, and any other byte that is not UTF-8
 * shows as U+FFFD. Null when the bytes are binary by the rule the text route
 * refuses a file by (`runtime.md` § What `demi file view` shows): a NUL byte
 * in their first 8 KiB. The media types that rule also names have their own
 * viewers, chosen by the file's name before its text is read.
 */
export function decodeTextStart(bytes: Uint8Array): string | null {
  if (bytes.subarray(0, 8 * 1024).includes(0))
    return null
  // Streaming keeps a sequence the cut splits for a next chunk that never comes.
  return new TextDecoder('utf-8').decode(bytes, { stream: true })
}

/** What a view shows of a text file too large to read whole: its first bytes as text, and how much it shows of how much. */
export type TextStart =
  | { phase: 'loading' }
  | { phase: 'ready'; text: string; shown: number; size: number }
  /** Not text, or its bytes could not be read: the view shows the file's card. */
  | { phase: 'card' }

/**
 * The first `TEXT_FILE_BYTES` of the text file at `path`, while `path` is
 * not null, read again when the file's version changes: a log that grows
 * shows its start anew, as its source reports the change. The read ends when
 * the path changes or the calling scope ends.
 */
export function useTextStart(
  contents: () => FileContents | undefined,
  path: () => string | null,
): ComputedRef<TextStart | null> {
  const description = useShowing(contents, path, (shown, at) => shown.showDescription(at))
  const state = shallowRef<TextStart>({ phase: 'loading' })
  const version = computed(() => {
    const entry = description.entry.value
    if (!entry || entry.value === undefined)
      return null
    return { size: entry.value.size, version: entry.value.version }
  })
  /** The file could not be described, as while its Host is away: the card shows until it can be. */
  const undescribed = computed(() => {
    const entry = description.entry.value
    return entry !== null && entry.value === undefined && entry.failure !== null
  })
  watch([contents, path, version, undescribed], ([shown, at, file, failed], _previous, onCleanup) => {
    if (!shown?.readStart || at === null || file === null) {
      state.value = shown?.readStart && !failed ? { phase: 'loading' } : { phase: 'card' }
      return
    }
    const reading = new AbortController()
    onCleanup(() => reading.abort())
    shown.readStart(at, TEXT_FILE_BYTES, { version: file.version ?? undefined, signal: reading.signal })
      .then((bytes) => {
        const text = decodeTextStart(bytes)
        state.value = text === null ? { phase: 'card' } : { phase: 'ready', text, shown: bytes.length, size: file.size }
      })
      .catch(() => {
        // A read cut short by the next one, or one that failed: the card says what the file is.
        if (!reading.signal.aborted)
          state.value = { phase: 'card' }
      })
  }, { immediate: true })
  return computed(() => (path() === null ? null : state.value))
}
