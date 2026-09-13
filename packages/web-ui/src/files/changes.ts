import type { ShellFileChange } from '@demicodes/agent'
import { baseName } from './paths'
import type { TreeRow } from './tree'

/**
 * The changes a conversation made to its workspace, as the change view
 * shows them: the files with their kinds and line counts, and both sides of
 * any one of them on request. A host maps its own source (the backend's
 * working-tree compare, a fixture) onto this. Paths are relative to the
 * workspace root.
 */
export interface ChangeSetSource {
  files: readonly ShellFileChange[]
  /** Both sides of one file: empty `original` for an added file, empty `modified` for a deleted one. */
  read(path: string, signal?: AbortSignal): Promise<{ original: string; modified: string }>
}

/**
 * Where a change view's diffs come from: files picked from the conversation
 * (each a snapshot around one tool call, nothing to do with git) or the
 * workspace's uncommitted changes against its last commit.
 */
export type ChangeMode = 'conversation' | 'uncommitted'

export const CHANGE_MODES: readonly ChangeMode[] = ['conversation', 'uncommitted']

/** A change set per mode; the change view switches between them. */
export type ChangeSources = Record<ChangeMode, ChangeSetSource>

/** A change set with nothing in it, for a conversation nothing has been picked from yet. */
export const emptyChangeSet: ChangeSetSource = {
  files: [],
  read: () => Promise.reject(new Error('No change recorded')),
}

/** The mode a new change tab opens in: what was picked from the conversation, if anything, else the working tree. */
export function changeModeToOpen(changes: ChangeSources): ChangeMode {
  return changes.conversation.files.length > 0 ? 'conversation' : 'uncommitted'
}

/** Lines added and removed across every file. */
export function changeTotals(files: readonly ShellFileChange[]): { added: number; removed: number } {
  let added = 0
  let removed = 0
  for (const file of files) {
    added += file.added
    removed += file.removed
  }
  return { added, removed }
}

/** One row of the change tree: a directory on the way to changed files (no change), or a changed file. */
export interface ChangeTreeRow extends TreeRow {
  change: ShellFileChange | null
}

/**
 * The changed files as a tree, directories first at every level and sorted
 * by name, with the directories in `folded` closed: their rows stay, what is
 * under them does not.
 */
export function changeTreeRows(files: readonly ShellFileChange[], folded: ReadonlySet<string>): ChangeTreeRow[] {
  interface Node {
    dirs: Map<string, Node>
    files: ShellFileChange[]
  }
  const root: Node = { dirs: new Map(), files: [] }
  for (const file of files) {
    const segments = file.path.split('/').filter(Boolean)
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
    for (const name of [...node.dirs.keys()].sort((a, b) => a.localeCompare(b))) {
      const path = prefix ? `${prefix}/${name}` : name
      const open = !folded.has(path)
      rows.push({ path, name, isDirectory: true, depth, parent, open, change: null })
      if (open) {
        walk(node.dirs.get(name)!, path, depth + 1, path)
      }
    }
    for (const change of [...node.files].sort((a, b) => baseName(a.path).localeCompare(baseName(b.path)))) {
      rows.push({ path: change.path, name: baseName(change.path), isDirectory: false, depth, parent, open: false, change })
    }
  }
  walk(root, '', 0, null)
  return rows
}
