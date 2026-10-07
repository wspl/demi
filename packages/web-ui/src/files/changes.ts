import { editedFileSchema, type EditCopies } from '@demicodes/protocol'
import { z } from 'zod'
import { compareFileNames } from './file-browser-state'
import { baseName } from '@demicodes/utils'
import type { Showing } from './file-cache'
import type { TreeRow } from './tree'
import type { FileContents, FileWatchNote } from './types'

/**
 * One changed file as the change view lists it: its path, how it changed,
 * and its line counts. A working-tree listing has every kind; a call's
 * retained edit (`EditedFile`) is one of these with `added` or
 * `modified` only.
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
  /** What the source says of the Host's watch; null or absent without one. */
  readonly watch?: FileWatchNote | null
  /**
   * Shows both sides of one file until released: empty `original` for an
   * added file, empty `modified` for a deleted one. A side that is not text
   * fails with kind `binary` or `too-large`.
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

/** One file picked under a shell call, independent of the call's other files. */
export const callEditSelectionSchema = z.object({
  commandId: z.string(),
  file: editedFileSchema,
})
export type CallEditSelection = z.infer<typeof callEditSelectionSchema>

export interface CallChangeSource extends CallEditSelection {
  /** The two sides of segment `edit`; null when it has no copies or one of them is gone. */
  read(edit: number, signal?: AbortSignal): Promise<ChangeSides | null>
}

export function callChangeSource(selection: CallEditSelection, read: ReadCallChange): CallChangeSource {
  return {
    ...selection,
    read: async (edit, signal) => {
      const copies = selection.file.edits[edit]?.copies
      return copies ? read(copies, signal) : null
    },
  }
}

/** What an empty change set says in place of its files, by why it is empty. */
export function emptyChangeSetText(source: ChangeSetSource): string {
  if (source.unavailable === 'no-repository') {
    return 'Not a git repository.'
  }
  if (source.failure) {
    return 'Could not list the changes.'
  }
  return 'The working tree matches the last commit.'
}

/**
 * Where a change view's diffs come from: files picked from the conversation
 * (each a snapshot around one tool call, nothing to do with git) or the
 * workspace's uncommitted changes against its last commit.
 */
export type ChangeMode = 'conversation' | 'uncommitted'

/** A live working-tree list and, independently, one retained file edit. */
export interface ChangeSources {
  uncommitted: ChangeSetSource
  conversation: CallChangeSource | null
}

/** The working-tree placeholder when no workspace is available: it lists nothing, so no side is shown. */
export const emptyChangeSet: ChangeSetSource = {
  files: [],
  showSides: () => ({
    entry: { value: undefined, failure: { kind: 'not-found', message: 'No change recorded' }, reading: false },
    retry: () => {},
    release: () => {},
  }),
}

/** One row of the change tree: a directory on the way to changed files (no change), or a changed file. */
export interface ChangeTreeRow extends TreeRow {
  change: WorkingTreeChange | null
}

/**
 * The changed files as a tree, directories first at every level and sorted
 * by name, with the directories in `folded` closed: their rows stay, what is
 * under them does not.
 */
export function changeTreeRows(files: readonly WorkingTreeChange[], folded: ReadonlySet<string>): ChangeTreeRow[] {
  interface Node {
    dirs: Map<string, Node>
    files: WorkingTreeChange[]
  }
  const root: Node = { dirs: new Map(), files: [] }
  for (const file of files) {
    const segments = file.path.split('/')
    let node = root
    for (const segment of segments.slice(0, -1)) {
      let child = node.dirs.get(segment)
      if (!child) {
        child = { dirs: new Map(), files: [] }
        node.dirs.set(segment, child)
      }
      node = child
    }
    node.files.push(file)
  }
  const rows: ChangeTreeRow[] = []
  const walk = (node: Node, prefix: string, depth: number, parent: string | null): void => {
    for (const name of [...node.dirs.keys()].sort(compareFileNames)) {
      const path = prefix ? `${prefix}/${name}` : name
      const open = !folded.has(path)
      rows.push({ path, name, isDirectory: true, depth, parent, open, change: null })
      if (open) {
        walk(node.dirs.get(name)!, path, depth + 1, path)
      }
    }
    for (const change of [...node.files].sort((a, b) => compareFileNames(baseName(a.path), baseName(b.path)))) {
      rows.push({ path: change.path, name: baseName(change.path), isDirectory: false, depth, parent, open: false, change })
    }
  }
  walk(root, '', 0, null)
  return rows
}
