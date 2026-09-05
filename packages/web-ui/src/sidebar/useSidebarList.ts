import { computed, ref, type Ref } from 'vue'
import { conversationIds, conversationIdsBetween, entryIndex, stepEntry, type ListEntry } from './list-model'

export interface SidebarListActions {
  open: (id: string) => void
  toggleFold: (projectId: string) => void
  fold: (projectId: string, folded: boolean) => void
  rename: (id: string) => void
  remove: (ids: string[]) => void
  togglePin: (ids: string[]) => void
}

/**
 * Selection, focus, and keys for the list: click selects and opens, ⌘/Ctrl-click toggles, Shift-click
 * ranges over rows; arrows move the focus and the selection with it, Shift+arrows extend, Space toggles,
 * Enter opens (or folds a project), ←/→ fold and unfold, ⌘A selects all, Esc collapses the selection to
 * the open conversation, F2 renames, ⌫ deletes, ⌘⇧P pins. Focus shows only after keyboard use.
 */
export function useSidebarList(entries: Ref<ListEntry[]>, openId: Ref<string | null>, actions: SidebarListActions) {
  const selected = ref<Set<string>>(new Set())
  const focusedId = ref<string | null>(null)
  const anchorId = ref<string | null>(null)
  const keyboardNav = ref(false)

  const selectedIds = computed(() => [...selected.value])
  const selectionCount = computed(() => selected.value.size)

  function isSelected(id: string): boolean {
    return selected.value.has(id)
  }

  function replaceSelection(ids: readonly string[]): void {
    selected.value = new Set(ids)
  }

  function selectOnly(id: string): void {
    replaceSelection([id])
    anchorId.value = id
    focusedId.value = id
  }

  function toggle(id: string): void {
    const next = new Set(selected.value)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    selected.value = next
    anchorId.value = id
    focusedId.value = id
  }

  function rangeTo(id: string): void {
    const from = anchorId.value ?? id
    replaceSelection(conversationIdsBetween(entries.value, from, id))
    focusedId.value = id
  }

  /** Selection collapses to the open conversation, which is what a plain list shows. */
  function clearSelection(): void {
    replaceSelection(openId.value ? [openId.value] : [])
    anchorId.value = openId.value
  }

  function selectAll(): void {
    replaceSelection(conversationIds(entries.value))
  }

  /** The rows a menu opened on `id` acts on: the selection when the row is in it, else the row alone. */
  function targetIds(id: string): string[] {
    if (!selected.value.has(id)) selectOnly(id)
    return [...selected.value]
  }

  function onRowClick(id: string, event: MouseEvent): void {
    keyboardNav.value = false
    if (event.shiftKey) {
      rangeTo(id)
      return
    }
    if (event.metaKey || event.ctrlKey) {
      toggle(id)
      return
    }
    selectOnly(id)
    actions.open(id)
  }

  function onRowContextMenu(id: string): void {
    keyboardNav.value = false
    if (!selected.value.has(id)) selectOnly(id)
    focusedId.value = id
  }

  function onProjectClick(projectId: string): void {
    keyboardNav.value = false
    focusedId.value = projectId
    actions.toggleFold(projectId)
  }

  function focusEntry(entry: ListEntry | undefined, extend: boolean): void {
    if (!entry) return
    focusedId.value = entry.id
    if (entry.kind !== 'conversation') return
    if (extend) rangeTo(entry.id)
    else selectOnly(entry.id)
  }

  function focusedEntry(): ListEntry | undefined {
    const index = entryIndex(entries.value, focusedId.value)
    return index >= 0 ? entries.value[index] : undefined
  }

  function onKeydown(event: KeyboardEvent): void {
    const meta = event.metaKey || event.ctrlKey
    const focused = focusedEntry()
    const handled = (() => {
      switch (event.key) {
        case 'ArrowDown':
          focusEntry(meta ? entries.value[entries.value.length - 1] : stepEntry(entries.value, focusedId.value, 1), event.shiftKey)
          return true
        case 'ArrowUp':
          focusEntry(meta ? entries.value[0] : stepEntry(entries.value, focusedId.value, -1), event.shiftKey)
          return true
        case 'Home':
          focusEntry(entries.value[0], event.shiftKey)
          return true
        case 'End':
          focusEntry(entries.value[entries.value.length - 1], event.shiftKey)
          return true
        case 'ArrowRight':
          if (focused?.kind === 'project') actions.fold(focused.id, false)
          return focused?.kind === 'project'
        case 'ArrowLeft':
          if (focused?.kind === 'project') {
            actions.fold(focused.id, true)
            return true
          }
          if (focused?.kind === 'conversation' && focused.projectId) {
            actions.fold(focused.projectId, true)
            focusedId.value = focused.projectId
            return true
          }
          return false
        case 'Enter':
          if (focused?.kind === 'project') actions.toggleFold(focused.id)
          else if (focused?.kind === 'conversation') actions.open(focused.id)
          return focused !== undefined
        case ' ':
          if (focused?.kind === 'conversation') toggle(focused.id)
          return focused?.kind === 'conversation'
        case 'Escape':
          clearSelection()
          return true
        case 'F2':
          if (focused?.kind === 'conversation') actions.rename(focused.id)
          return focused?.kind === 'conversation'
        case 'Backspace':
        case 'Delete':
          if (selected.value.size > 0) actions.remove([...selected.value])
          return selected.value.size > 0
        case 'a':
        case 'A':
          if (!meta) return false
          selectAll()
          return true
        case 'p':
        case 'P':
          if (!meta || !event.shiftKey || selected.value.size === 0) return false
          actions.togglePin([...selected.value])
          return true
        default:
          return false
      }
    })()
    if (!handled) return
    keyboardNav.value = true
    event.preventDefault()
  }

  /** Rows that left the list leave the selection too. */
  function prune(): void {
    const present = new Set(entries.value.map((entry) => entry.id))
    const next = [...selected.value].filter((id) => present.has(id))
    if (next.length !== selected.value.size) replaceSelection(next)
    if (focusedId.value && !present.has(focusedId.value)) focusedId.value = null
    if (anchorId.value && !present.has(anchorId.value)) anchorId.value = null
  }

  return {
    selected,
    selectedIds,
    selectionCount,
    focusedId,
    keyboardNav,
    isSelected,
    selectOnly,
    clearSelection,
    targetIds,
    onRowClick,
    onRowContextMenu,
    onProjectClick,
    onKeydown,
    prune,
  }
}
