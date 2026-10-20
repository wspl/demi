import { z } from 'zod'
import { isAbsoluteHostPath, joinPath, type IntentPayloads } from '@demicodes/plugin-sdk'

/**
 * A file the view showed, with the directory its tree was rooted at then
 * (absent: the conversation's), which Back and Forward return to.
 */
const fileEntrySchema = z.object({
  path: z.string(),
  root: z.string().optional(),
})
type FileEntry = z.infer<typeof fileEntrySchema>

/**
 * What the File view shows: one file of the conversation's Host, by its
 * absolute path; the files Back and Forward return to; and the directory it was
 * opened for.
 */
export const fileDataSchema = z.object({
  /** Empty while it shows no file. */
  path: z.string(),
  /**
   * The directory the view was opened for, whose tree it shows: the working
   * directory of the agent whose link, file change or folder opened it; absent,
   * the conversation's.
   */
  root: z.string().optional(),
  /** The line the file was opened at, which the view scrolls to and marks for a moment; absent for none. */
  line: z.number().int().positive().optional(),
  /** The files shown before, latest last, each with the directory it was shown under. */
  back: z.array(fileEntrySchema),
  /** The files left by Back, latest last. */
  forward: z.array(fileEntrySchema),
  /**
   * Show in Files asked to see the file in the view's tree: the view shows
   * the tree with the file selected and in view, once, and drops it.
   */
  tree: z.literal(true).optional(),
})
export type FileData = z.infer<typeof fileDataSchema>

/**
 * The file the view shows for `path`, read from `root`, the directory it was
 * opened for: a full path as its Host writes it, a Windows one such as
 * `C:\Users\alex\chart.png` included, is that file.
 */
export function shownPath(root: string, path: string): string {
  return isAbsoluteHostPath(path) ? path : joinPath(root, path)
}

/** The view as a conversation's panel first shows it: no file, nothing to go back to. */
export function firstFileData(): FileData {
  return { path: '', back: [], forward: [] }
}

/** The directory a view or an entry keeps, as a field that is absent for the conversation's. */
function rootOf(of: { root?: string }): { root?: string } {
  return of.root === undefined ? {} : { root: of.root }
}

/** What the view shows now, as an entry. */
function entryOf(data: FileData): FileEntry {
  return { path: data.path, ...rootOf(data) }
}

/**
 * The view showing `path`, at `line` when one is named, opened for the
 * directory `root` (absent: the conversation's), the file it showed kept
 * for Back with its own directory; another line or directory of the file
 * shown moves within it.
 */
export function showFile(data: FileData, path: string, line?: number, root?: string): FileData {
  if (data.path === path && data.line === line && data.root === root) {
    return data
  }
  const at = { ...(line === undefined ? {} : { line }), ...(root === undefined ? {} : { root }) }
  if (data.path === path) {
    return { path, ...at, back: data.back, forward: data.forward }
  }
  return {
    path,
    ...at,
    back: data.path ? [...data.back, entryOf(data)] : data.back,
    forward: [],
  }
}

/** The view showing `path` as the `file` intent asks: in the tree too when `tree`, as Show in Files asks. */
export function openFile(data: FileData, payload: IntentPayloads['file']): FileData {
  const shown = showFile(data, payload.path, payload.line, payload.root)
  return payload.tree ? { ...shown, tree: true } : shown
}

/** The view as it is once its tree showed the file Show in Files asked for. */
export function treeShown(data: FileData): FileData {
  const { tree: _shown, ...rest } = data
  return rest
}

/** The view showing what it showed before, under the directory it had then, the current file kept for Forward. */
export function goBack(data: FileData): FileData {
  const previous = data.back.at(-1)
  if (previous === undefined) {
    return data
  }
  return { ...previous, back: data.back.slice(0, -1), forward: [...data.forward, entryOf(data)] }
}

/** The view showing what Back left, the current file kept for Back. */
export function goForward(data: FileData): FileData {
  const next = data.forward.at(-1)
  if (next === undefined) {
    return data
  }
  return { ...next, forward: data.forward.slice(0, -1), back: [...data.back, entryOf(data)] }
}
