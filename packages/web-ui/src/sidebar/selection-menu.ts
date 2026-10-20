import { Archive, ArrowRight, ChevronsDownUp, ChevronsUpDown, Copy, ListChecks, Pin, PinOff, SquarePen, TextCursorInput, Trash2 } from '@lucide/vue'
import type { ContextMenuEntry, ContextMenuGroup } from '../ui/context-menu'
import { t } from './locales/text'
import type { SidebarConversation } from './types'

/** What the conversations' commands do; the one who offers them supplies it. */
export interface SelectionActions {
  open: (id: string) => void
  rename: (id: string) => void
  copyId: (id: string) => void
  pin: (ids: string[], pinned: boolean) => void
  archive: (ids: string[]) => void
  /** Delete…: the host asks before anything goes. */
  delete: (ids: string[]) => void
}

/**
 * The commands on conversations, one menu wherever they are offered: the
 * sidebar's row menu, and on a phone the title sheet of the conversation shown
 * (ChatSession).
 *
 * - One row gets open, rename and copy ID.
 * - Any count gets pin, archive and delete, which asks first.
 * - A conversation enters or leaves a project only through where it runs, so
 *   there is no Move To.
 *
 * `shown`: the one target is the conversation shown, which Open would not change.
 */
export function selectionEntries(
  targets: readonly Pick<SidebarConversation, 'id' | 'pinned'>[],
  actions: SelectionActions,
  shown = false,
): ContextMenuGroup[] {
  const ids = targets.map((target) => target.id)
  const count = targets.length
  const single = count === 1 ? targets[0]! : null
  const allPinned = count > 0 && targets.every((target) => target.pinned)
  const first: ContextMenuEntry[] = []
  if (single) {
    if (!shown)
      first.push({ icon: ArrowRight, label: t('selection.open'), shortcut: '↵', run: () => actions.open(single.id) })
    first.push(
      { icon: TextCursorInput, label: t('selection.rename'), shortcut: 'F2', run: () => actions.rename(single.id) },
      { icon: Copy, label: t('selection.copyId'), run: () => actions.copyId(single.id) },
    )
  }
  first.push(
    {
      icon: allPinned ? PinOff : Pin,
      label: allPinned
        ? (count > 1 ? t('selection.unpinMany', { count }) : t('selection.unpin'))
        : (count > 1 ? t('selection.pinMany', { count }) : t('selection.pin')),
      shortcut: '⌘⇧P',
      run: () => actions.pin(ids, !allPinned),
    },
    { icon: Archive, label: count > 1 ? t('selection.archiveMany', { count }) : t('selection.archive'), run: () => actions.archive(ids) },
  )
  return [
    first,
    [{ icon: Trash2, label: count > 1 ? t('selection.deleteMany', { count }) : t('selection.delete'), danger: true, run: () => actions.delete(ids) }],
  ]
}

/** A project header's commands. */
export function projectEntries(
  state: { folded: boolean; count: number },
  actions: { create: () => void; toggleFold: () => void; selectAll: () => void; delete: () => void },
): ContextMenuGroup[] {
  return [
    [
      { icon: SquarePen, label: t('projectMenu.newHere'), run: actions.create },
      { icon: state.folded ? ChevronsUpDown : ChevronsDownUp, label: state.folded ? t('projectMenu.unfold') : t('projectMenu.fold'), shortcut: '↵', run: actions.toggleFold },
      { icon: ListChecks, label: t('projectMenu.select'), disabled: state.count === 0, run: actions.selectAll },
    ],
    // Asks before the project is deleted (`ProjectDeleteDialog`).
    [{ icon: Trash2, label: t('projectMenu.delete'), danger: true, run: actions.delete }],
  ]
}
