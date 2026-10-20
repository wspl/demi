import { Copy, Download, FolderTree, Hash, SquareArrowOutUpRight } from '@lucide/vue'
import type { ContextMenuEntry, ContextMenuGroup } from '../ui/context-menu'
import { copyId } from '../infra/copy-id'
import { t as ui } from '../ui/locales/text'
import { t } from './locales/text'

/**
 * The menus of a Host file and of a commit, each declared once.
 *
 * - A link that names the object offers all of it, Open first.
 * - The row the object has elsewhere offers the same less what the row already is:
 *   a file row in the tree no Open and no Show in Files, a commit row in Commits
 *   mode no Open.
 * - An action the place cannot take is left out.
 */

/** What a place can do with a file it names. */
export interface FileMenuActions {
  /** Acts as a click on the link does. */
  open?: () => void
  /** Opens the File view's tree with the file selected and in view. */
  showInFiles?: () => void
  /** Starts the download of the file; absent where the place serves no bytes. */
  download?: () => void
}

/** A file's menu: Open and Show in Files, then Copy Path, its absolute path on the Host, and Download. */
export function fileMenu(path: string, actions: FileMenuActions): ContextMenuGroup[] {
  const { open, showInFiles, download } = actions
  const opening: ContextMenuEntry[] = [
    ...(open ? [{ icon: SquareArrowOutUpRight, label: ui('contextMenu.open'), run: open }] : []),
    ...(showInFiles ? [{ icon: FolderTree, label: t('actions.showInFiles'), run: showInFiles }] : []),
  ]
  return [
    opening,
    [
      { icon: Copy, label: t('actions.copyPath'), run: () => copyId(path) },
      ...(download ? [{ icon: Download, label: t('actions.download'), run: download }] : []),
    ],
  ].filter((group) => group.length > 0)
}

/** What a place can do with a commit it names. */
export interface CommitMenuActions {
  /** Acts as a click on the link does. */
  open?: () => void
  /** The commit's full hash, which a link's abbreviation is read to; absent, no Copy Commit Hash. */
  hash?: () => string | Promise<string>
}

/** A commit's menu: Open, then Copy Commit Hash, its full hash. */
export function commitMenu(actions: CommitMenuActions): ContextMenuGroup[] {
  const { open, hash } = actions
  return [
    ...(open ? [[{ icon: SquareArrowOutUpRight, label: ui('contextMenu.open'), run: open }]] : []),
    ...(hash ? [[{ icon: Hash, label: t('changes.copyCommitHash'), run: () => copyId(hash()) }]] : []),
  ]
}
