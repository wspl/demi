import { h } from 'vue'
import { Archive, ArrowRight, ChevronsDownUp, ChevronsUpDown, Copy, FolderInput, ListChecks, NotebookPen, Pin, PinOff, SquarePen, TextCursorInput, Trash2 } from '@lucide/vue'
import type { ContextMenuEntry, ContextMenuGroup } from '../ui/context-menu'
import { namedKeyCap, shortcutFor } from '../ui/shortcut'
import { t } from './locales/text'
import SavedScopeMenu from './SavedScopeMenu.vue'
import WorkspaceMoveMenu from '../workspaces/WorkspaceMoveMenu.vue'
import { t as workspaceText } from '../workspaces/locales/text'
import type { WorkspaceView } from '../workspaces/workspaces'
import type { SavedScope, SidebarConversation } from './types'

/** The keys that pin or unpin the selected conversations in the list (`useSidebarList`), which the menu shows. */
export function pinKeys(): string {
  return shortcutFor({ mac: '⇧⌘P', pc: 'Ctrl+Shift+P' })
}

/** What the conversations' commands do; the one who offers them supplies it. */
export interface SelectionActions {
  open: (id: string) => void
  rename: (id: string) => void
  copyId: (id: string) => void
  pin: (ids: string[], pinned: boolean) => void
  archive: (ids: string[]) => void
  /** Delete…: the host asks before anything goes. */
  delete: (ids: string[]) => void
  /** Move To: the workspaces, and what a choice of one does; absent, the menu has no Move To. */
  moveTo?: WorkspaceMove
}

/** Move To's workspaces and its choice, for conversations or a project. */
export interface WorkspaceMove {
  workspaces: readonly WorkspaceView[]
  /** The workspace the targets are in. */
  currentId: string | null
  run: (workspaceId: string) => void
}

/** Move To, with the workspaces in its submenu. */
function moveToEntry(move: WorkspaceMove): ContextMenuEntry {
  return {
    icon: FolderInput,
    label: workspaceText('moveTo'),
    submenu: () => h(WorkspaceMoveMenu, { workspaces: move.workspaces, currentId: move.currentId, onChoose: move.run }),
  }
}

/**
 * The commands on conversations, one menu wherever they are offered: the
 * sidebar's row menu, and on a phone the title sheet of the conversation shown
 * (ChatSession).
 *
 * - One row gets open, rename and copy ID.
 * - Any count gets pin, archive and delete, which asks first.
 * - A conversation enters or leaves a project only through where it runs;
 *   Move To moves it to another workspace.
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
      first.push({ icon: ArrowRight, label: t('selection.open'), shortcut: namedKeyCap('Enter'), run: () => actions.open(single.id) })
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
      shortcut: pinKeys(),
      run: () => actions.pin(ids, !allPinned),
    },
    { icon: Archive, label: count > 1 ? t('selection.archiveMany', { count }) : t('selection.archive'), run: () => actions.archive(ids) },
  )
  if (actions.moveTo) {
    first.push(moveToEntry(actions.moveTo))
  }
  return [
    first,
    [{ icon: Trash2, label: count > 1 ? t('selection.deleteMany', { count }) : t('selection.delete'), danger: true, run: () => actions.delete(ids) }],
  ]
}

/** A project header's commands. */
export function projectEntries(
  state: { folded: boolean; count: number; savedScope: SavedScope; savedCount: number },
  actions: {
    create: () => void
    toggleFold: () => void
    selectAll: () => void
    /** A choice of Saved Messages' submenu, which differs from the project's way. */
    savedScope: (scope: SavedScope) => void
    delete: () => void
    /** Move To; absent, the menu has none. */
    moveTo?: WorkspaceMove
  },
): ContextMenuGroup[] {
  return [
    [
      { icon: SquarePen, label: t('projectMenu.newHere'), run: actions.create },
      { icon: state.folded ? ChevronsUpDown : ChevronsDownUp, label: state.folded ? t('projectMenu.unfold') : t('projectMenu.fold'), shortcut: namedKeyCap('Enter'), run: actions.toggleFold },
      { icon: ListChecks, label: t('projectMenu.select'), disabled: state.count === 0, run: actions.selectAll },
      {
        icon: NotebookPen,
        label: t('projectMenu.saved'),
        submenu: () => h(SavedScopeMenu, {
          scope: state.savedScope,
          hasConversation: state.count > 0,
          savedCount: state.savedCount,
          onChoose: actions.savedScope,
        }),
      },
      ...(actions.moveTo ? [moveToEntry(actions.moveTo)] : []),
    ],
    // Asks before the project is deleted (`ProjectDeleteDialog`).
    [{ icon: Trash2, label: t('projectMenu.delete'), danger: true, run: actions.delete }],
  ]
}
