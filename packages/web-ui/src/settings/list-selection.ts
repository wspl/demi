import { computed, ref, type Ref } from 'vue'

/**
 * Which entries of a settings list are selected, for a list that acts on
 * several at once.
 *
 * - In a rail, a click selects one; ⌘-click (Ctrl-click off a Mac) adds or
 *   drops one; Shift-click selects every entry from the last one clicked, as
 *   Finder's list does.
 * - In a list whose rows open a page, as on a phone, `checking` turns every
 *   tap into adding or dropping one, as a list's Select mode does on iOS.
 * - A table or a list with checkboxes toggles through `toggle` and `toggleAll`.
 *
 * `ids` are the entries in the order the list shows them, the only order a
 * Shift-click reads. An id the list no longer shows leaves the selection.
 */
export function useListSelection(ids: Ref<readonly string[]>, initial: readonly string[] = []) {
  const picked = ref<Set<string>>(new Set(initial))
  const anchor = ref<string | null>(initial[0] ?? null)
  const checking = ref(false)

  const selected = computed<ReadonlySet<string>>(() => {
    const shown = new Set(ids.value)
    return new Set([...picked.value].filter((id) => shown.has(id)))
  })

  function only(id: string | null): void {
    picked.value = new Set(id === null ? [] : [id])
    anchor.value = id
  }

  function toggle(id: string, on = !picked.value.has(id)): void {
    const next = new Set(picked.value)
    if (on) {
      next.add(id)
    } else {
      next.delete(id)
    }
    picked.value = next
    anchor.value = id
  }

  /** Every shown entry, or none once every one is. */
  function toggleAll(): void {
    const all = ids.value.length > 0 && ids.value.every((id) => picked.value.has(id))
    picked.value = new Set(all ? [] : ids.value)
  }

  /** A click on `id` in a rail. */
  function pick(event: MouseEvent, id: string): void {
    if (checking.value || event.metaKey || event.ctrlKey) {
      toggle(id)
      return
    }
    const from = anchor.value === null ? -1 : ids.value.indexOf(anchor.value)
    if (event.shiftKey && from >= 0) {
      const to = ids.value.indexOf(id)
      const [start, end] = from < to ? [from, to] : [to, from]
      picked.value = new Set(ids.value.slice(start, end + 1))
      return
    }
    only(id)
  }

  function check(on: boolean): void {
    checking.value = on
    picked.value = new Set()
  }

  return { selected, checking, pick, only, toggle, toggleAll, check }
}
