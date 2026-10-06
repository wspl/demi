import { computed, shallowRef, watch, type ComputedRef } from 'vue'
import type { Showing, ShownEntry } from './file-cache'

/** One kept entry a component shows (`useShowing`). */
export interface ShownState<T> {
  /** The entry shown now; null while there is nothing to show. */
  readonly entry: ComputedRef<ShownEntry<T> | null>
  /** Reads the shown entry again now, as Retry does. */
  retry(): void
}

/**
 * Shows the entry at `path` of what `owner` gives, through `open`, while the
 * calling component's scope lives: when either changes, the entry they name
 * now is shown and the last one let go. Nothing shows while either is null,
 * or `open` answers null.
 */
export function useShowing<O, T>(
  owner: () => O | null | undefined,
  path: () => string | null | undefined,
  open: (owner: O, path: string) => Showing<T> | null | undefined,
): ShownState<T> {
  const current = shallowRef<Showing<T> | null>(null)
  watch([owner, path], ([shownOwner, shownPath], _previous, onCleanup) => {
    const showing = shownOwner == null || shownPath == null ? null : open(shownOwner, shownPath) ?? null
    current.value = showing
    // Runs before the next entry shows, and when the scope ends.
    onCleanup(() => showing?.release())
  }, { immediate: true })
  return {
    entry: computed(() => current.value?.entry ?? null),
    retry: () => current.value?.retry(),
  }
}
