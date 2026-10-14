import type { EditCopies } from '@demicodes/protocol'
import { compareFileNames } from './file-browser-state'
import { baseName } from '@demicodes/utils'
import type { Showing } from './file-cache'
import type { TreeRow } from './tree'
import type { RequestChangeSource, RequestFile } from './request-changes'
import { relativePath, resolveHostPath } from './paths'
import type { FileContents } from './types'

/**
 * One changed file as the change view and the file pills list it: its
 * path, how it changed, and its line counts, both zero when they are not
 * known.
 */
export interface ChangeFile {
  path: string
  kind: 'added' | 'modified' | 'deleted' | 'renamed'
  /** The path before a rename. */
  from?: string
  added: number
  removed: number
}

/**
 * A path of the working tree that `git status` lists: how it differs from
 * the last commit, and git's two status letters for it (`runner.md`
 * § Working tree), which mark it the way VS Code's Git does (`gitMark`).
 */
export interface WorkingTreeChange extends ChangeFile {
  status: string
}

/** The workspace's uncommitted files and their current differences from HEAD. */
export interface ChangeSetSource {
  files: readonly WorkingTreeChange[]
  /** The list stopped at the host's limit; there are more changed files than it holds. */
  truncated?: boolean
  /** Why there is nothing to list, when the reason is not that nothing changed. */
  unavailable?: 'no-repository' | null
  /** A new list is on its way; `files` are the last one until it lands. */
  refreshing?: boolean
  /** The last listing failed, in these words; `files` are the last list that succeeded. */
  failure?: string | null
  /** Lists the working tree again. */
  refresh?(): void
  /**
   * Why the Host does not watch the list, while it says so: the view shows
   * it as last read, with Refresh; null or absent while it watches.
   */
  readonly unwatched?: string | null
  /**
   * Shows both sides of one file until released: empty `original` for an
   * added file, empty `modified` for a deleted one. A side that is not text
   * is the answer `binary` or `too-large`.
   */
  showSides(path: string): Showing<ChangeSides>
  /**
   * Each file as the last commit has it, by the path relative to the
   * workspace, for previews and Download; absent when the source serves no
   * bytes.
   */
  committed?: FileContents
}

export interface ChangeSides { original: string; modified: string }
/**
 * Reads one edit segment's two sides by the blobs its copies name
 * (`edit-tracking.md` § Edit copies); null when one of them is gone.
 */
export type ReadCallChange = (copies: EditCopies, signal?: AbortSignal) => Promise<ChangeSides | null>

/** What an empty change set says in place of its files, by why it is empty. */
export function emptyChangeSetText(source: ChangeSetSource): string {
  if (source.unavailable === 'no-repository') {
    return 'Not a Git repository'
  }
  if (source.failure) {
    return 'Could not list the changes.'
  }
  return 'No changes'
}

/**
 * Where a change view's diffs come from: a request of the conversation (the
 * files its calls changed, nothing to do with git) or the workspace's
 * uncommitted changes against its last commit.
 */
export type ChangeMode = 'conversation' | 'uncommitted'

/** A live working-tree list and, independently, the request the view shows; null while it shows none. */
export interface ChangeSources {
  uncommitted: ChangeSetSource
  conversation: RequestChangeSource | null
}

/** The working-tree placeholder when no workspace is available: it lists nothing, so no side is shown. */
export const emptyChangeSet: ChangeSetSource = {
  files: [],
  showSides: () => ({
    entry: { value: undefined, answer: 'gone', failure: null, reading: false, unwatched: null },
    retry: () => Promise.resolve(),
    release: () => {},
    away: () => {},
    back: () => {},
  }),
}

/**
 * A changed file as the Change view's tree shows it, in either mode: its
 * key, its path within the tree, how it changed, its line counts when they
 * are known, and in Uncommitted mode git's status letters for it.
 */
export interface ChangeTreeFile {
  /** The file's key: the path the view selects it by. */
  key: string
  /** Its path from the tree's root, with `/` separators. */
  path: string
  kind: ChangeFile['kind']
  /** The path before a rename, as the file's tooltip names it. */
  from?: string
  /** Absent when they are not known. */
  lines?: { added: number; removed: number }
  /** Git's two status letters, in Uncommitted mode (`gitMark`). */
  status?: string
}

/**
 * A request's files as the change tree shows them: each by its path from
 * `root`, the workspace, in the spelling of the root's Host, so a Windows
 * file `C:\Users\u\shop\src\cart.ts` under `C:\Users\u\shop` is in `src`.
 */
export function requestTreeFiles(root: string, files: readonly RequestFile[]): ChangeTreeFile[] {
  return files.map((file) => ({
    key: file.path,
    path: relativePath(root, resolveHostPath(root, file.path)),
    kind: file.kind,
    ...(file.from ? { from: file.from } : {}),
    ...(file.lines ? { lines: file.lines } : {}),
  }))
}

/**
 * One row of the change tree: a folder on the way to changed files, its
 * name the folders it joins, such as `src/web/app`, or a changed file,
 * whose row's path is its key.
 */
export interface ChangeTreeRow extends TreeRow {
  file: ChangeTreeFile | null
}

/**
 * The changed files as a tree, as VS Code's Source Control shows changes
 * as a tree (`edit-tracking.md` § What the conversation shows): only the
 * folders that hold a changed file, folders first and then files at every
 * level, each sorted by name, and a folder whose only content is one folder
 * joined with it into one row, `src/web/app`. The folders in `folded`, by
 * their path from the root, are closed: their rows stay, what is under
 * them does not.
 */
export function changeTreeRows(files: readonly ChangeTreeFile[], folded: ReadonlySet<string>): ChangeTreeRow[] {
  interface Folder {
    folders: Map<string, Folder>
    files: ChangeTreeFile[]
  }
  const root: Folder = { folders: new Map(), files: [] }
  for (const file of files) {
    const segments = file.path.split('/').filter((segment) => segment !== '')
    let folder = root
    for (const segment of segments.slice(0, -1)) {
      let inner = folder.folders.get(segment)
      if (!inner) {
        inner = { folders: new Map(), files: [] }
        folder.folders.set(segment, inner)
      }
      folder = inner
    }
    folder.files.push(file)
  }
  const rows: ChangeTreeRow[] = []
  const walk = (folder: Folder, prefix: string, depth: number, parent: string | null): void => {
    for (const name of [...folder.folders.keys()].sort(compareFileNames)) {
      let inner = folder.folders.get(name)!
      let joined = name
      while (inner.files.length === 0 && inner.folders.size === 1) {
        const [next, only] = [...inner.folders][0]!
        joined = `${joined}/${next}`
        inner = only
      }
      const path = prefix ? `${prefix}/${joined}` : joined
      const open = !folded.has(path)
      rows.push({ path, name: joined, isDirectory: true, depth, parent, open, file: null })
      if (open) {
        walk(inner, path, depth + 1, path)
      }
    }
    for (const file of [...folder.files].sort((a, b) => compareFileNames(baseName(a.path), baseName(b.path)))) {
      rows.push({ path: file.key, name: baseName(file.path), isDirectory: false, depth, parent, open: false, file })
    }
  }
  walk(root, '', 0, null)
  return rows
}
