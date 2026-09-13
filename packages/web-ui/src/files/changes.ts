import type { ShellEditsView } from '@demicodes/agent/client'
import type { ShellFileChange } from '@demicodes/agent'
import { baseName, joinPath } from './paths'
import type { TreeRow } from './tree'

/**
 * The changes a conversation made to its workspace, as the change view
 * shows them: the files with their kinds and line counts, and both sides of
 * any one of them on request. A host maps its own source (the backend's
 * working-tree compare, a fixture) onto this. Paths are relative to the
 * workspace root for working-tree entries and absolute for retained call edits.
 */
export interface ChangeSetSource {
  files: readonly ShellFileChange[]
  identity?: string
  segments?(path: string): readonly { kept: boolean }[]
  /** The list stopped at the host's limit; there are more changed files than it holds. */
  truncated?: boolean
  /** Why there is nothing to list, when the reason is not that nothing changed. */
  unavailable?: 'no-repository' | null
  /** A new list is on its way; `files` are the last one until it lands. */
  refreshing?: boolean
  /** The last listing failed, in these words; `files` are the last list that succeeded. */
  failure?: string | null
  /** Lists again. Absent when the source has one fixed list, such as the conversation's picks. */
  refresh?(): void
  /** Both sides of one file: empty `original` for an added file, empty `modified` for a deleted one. */
  read(path: string, signal?: AbortSignal, edit?: number): Promise<ChangeSides | null>
}

export interface ChangeSides { original: string; modified: string }
export type ReadCallChange = (commandId: string, path: string, edit: number, signal?: AbortSignal) => Promise<ChangeSides | null>

/** A fixed call list, with retained contents loaded only for the selected segment. */
export function callChangeSet(call: ShellEditsView, read: ReadCallChange): ChangeSetSource {
  return {
    identity: call.commandId,
    files: call.files,
    truncated: call.filesTruncated,
    segments: (path) => call.files.find((file) => file.path === path)?.edits ?? [],
    read: (path, signal, edit = 0) => read(call.commandId, path, edit, signal),
  }
}

/** Recorded paths can be absolute on either supported path syntax. */
export function changeAbsolutePath(path: string, root: string): string {
  const normalized = path.replaceAll('\\', '/')
  return normalized.startsWith('/') || /^[A-Za-z]:\//.test(normalized)
    ? normalized
    : joinPath(root, normalized)
}

/** Display under-root paths relatively without changing their lookup identity. */
export function changeDisplayPath(path: string, root: string): string {
  const normalized = path.replaceAll('\\', '/')
  const prefix = root.replaceAll('\\', '/').replace(/\/$/, '') + '/'
  return normalized.startsWith(prefix) ? normalized.slice(prefix.length) : normalized
}

/** What an empty change set says in place of its files, by why it is empty. */
export function emptyChangeSetText(source: ChangeSetSource, mode: ChangeMode): string {
  if (mode === 'conversation') {
    return 'Click a changed file in the conversation to see its diff here.'
  }
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

export const CHANGE_MODES: readonly ChangeMode[] = ['uncommitted', 'conversation']

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

/** One row of the change tree: a directory on the way to changed files (no change), or a changed file. */
export interface ChangeTreeRow extends TreeRow {
  change: ShellFileChange | null
}

/**
 * The changed files as a tree, directories first at every level and sorted
 * by name, with the directories in `folded` closed: their rows stay, what is
 * under them does not.
 */
export function changeTreeRows(files: readonly ShellFileChange[], folded: ReadonlySet<string>, workspaceRoot = ''): ChangeTreeRow[] {
  interface Node {
    dirs: Map<string, Node>
    files: ShellFileChange[]
  }
  const root: Node = { dirs: new Map(), files: [] }
  for (const file of files) {
    const segments = changeDisplayPath(file.path, workspaceRoot).split('/').filter(Boolean)
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
    for (const change of [...node.files].sort((a, b) => baseName(changeDisplayPath(a.path, workspaceRoot)).localeCompare(baseName(changeDisplayPath(b.path, workspaceRoot))))) {
      rows.push({ path: change.path, name: baseName(changeDisplayPath(change.path, workspaceRoot)), isDirectory: false, depth, parent, open: false, change })
    }
  }
  walk(root, '', 0, null)
  return rows
}
