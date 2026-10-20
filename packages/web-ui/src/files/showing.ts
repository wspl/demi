import { computed, onScopeDispose, shallowRef, watch, type ComputedRef } from 'vue'
import type { Showing, ShownEntry } from './file-cache'
import { followOnScreen } from './on-screen'

/** One kept entry a component shows (`useShowing`). */
export interface ShownState<T> {
  /** The entry shown now; null while there is nothing to show. */
  readonly entry: ComputedRef<ShownEntry<T> | null>
  /** Reads the shown entry again now, as Retry does; it settles when that read ends, at once with nothing shown. */
  retry(): Promise<void>
}

/**
 * Shows the entry at `path` of what `owner` gives, through `open`, while the
 * calling component's scope lives.
 *
 * - When either changes, the entry they name now is shown and the last one let go.
 * - Nothing shows while either is null, or `open` answers null.
 * - It follows the entry only while the component's views are on screen.
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
  followOnScreen(() => [current.value])
  return {
    entry: computed(() => current.value?.entry ?? null),
    retry: () => current.value?.retry() ?? Promise.resolve(),
  }
}

/** Kept entries a component shows by path (`useShowings`). */
export interface ShownEntries<T> {
  /** The entry shown at each path. */
  readonly entries: ComputedRef<ReadonlyMap<string, ShownEntry<T>>>
  /** Reads the entry at `path` again now, as Retry does; at once when none is shown there. */
  retry(path: string): Promise<void>
}

/**
 * Shows the entry at each of `paths` of what `owner` gives, through `open`, while
 * the calling component's scope lives, as `useShowing` shows one.
 *
 * - A path that stays keeps its hold, and one that leaves is let go.
 * - A new owner shows them all anew.
 * - It follows the entries only while the component's views are on screen.
 */
export function useShowings<O, T>(
  owner: () => O | null | undefined,
  paths: () => readonly string[],
  open: (owner: O, path: string) => Showing<T>,
): ShownEntries<T> {
  const held = shallowRef<ReadonlyMap<string, Showing<T>>>(new Map())
  /** The owner `held` was shown from. */
  let heldOwner: O | null | undefined
  const wanted = computed(() => [...new Set(paths())])
  // Compared by the paths they name, so a render that names the same ones holds what it held.
  const named = computed(() => [...wanted.value].sort().join('\0'))
  watch([owner, named], ([shownOwner]) => {
    const kept = shownOwner === heldOwner ? held.value : new Map<string, Showing<T>>()
    const next = new Map<string, Showing<T>>()
    if (shownOwner != null) {
      for (const path of wanted.value)
        next.set(path, kept.get(path) ?? open(shownOwner, path))
    }
    // The new ones are shown before the old are let go, so an entry both hold keeps its subscriptions.
    const holding = new Set(next.values())
    for (const showing of held.value.values()) {
      if (!holding.has(showing))
        showing.release()
    }
    heldOwner = shownOwner
    held.value = next
  }, { immediate: true })
  onScopeDispose(() => {
    for (const showing of held.value.values())
      showing.release()
  })
  followOnScreen(() => [...held.value.values()])
  return {
    entries: computed(() => new Map([...held.value].map(([path, showing]) => [path, showing.entry]))),
    retry: (path) => held.value.get(path)?.retry() ?? Promise.resolve(),
  }
}
