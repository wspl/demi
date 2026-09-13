import type { ShellFileChange } from '@demicodes/agent'
import { baseName } from './paths'

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

/** One row of the change tree: a directory on the way to changed files, or a changed file. */
export type ChangeTreeRow =
  | { kind: 'directory'; path: string; name: string; depth: number }
  | { kind: 'file'; path: string; name: string; depth: number; change: ShellFileChange }

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
  const walk = (node: Node, prefix: string, depth: number): void => {
    for (const name of [...node.dirs.keys()].sort((a, b) => a.localeCompare(b))) {
      const path = prefix ? `${prefix}/${name}` : name
      rows.push({ kind: 'directory', path, name, depth })
      if (!folded.has(path)) {
        walk(node.dirs.get(name)!, path, depth + 1)
      }
    }
    for (const change of [...node.files].sort((a, b) => baseName(a.path).localeCompare(baseName(b.path)))) {
      rows.push({ kind: 'file', path: change.path, name: baseName(change.path), depth, change })
    }
  }
  walk(root, '', 0)
  return rows
}
