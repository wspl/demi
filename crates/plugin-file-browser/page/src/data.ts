import { ref } from 'vue'
import { z } from 'zod'
import { definePlace, isAbsoluteHostPath, joinPath, type IntentPayloads } from '@demicodes/plugin-sdk'

/**
 * What the File view shows: one file of the conversation's Host, by its
 * absolute path, and the directory it was opened for. The files it showed
 * before are the web browser's history, through the page's address
 * (`filePlace`).
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
  /**
   * Whether a Markdown or SVG file shows its preview or its source, as the
   * address records it for the file shown; absent, the view the user last
   * chose anywhere (`fileView`), which the address records with the file.
   */
  view: z.enum(['preview', 'source']).optional(),
  /**
   * A folder the crumb row went to, found and selected in the view's tree
   * while the file stays shown, until another file opens.
   */
  folder: z.string().optional(),
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

/**
 * Markdown and SVG: the view the user last chose, Preview or Source, for the
 * page's lifetime, in which a file the address names no view for shows.
 */
export const fileView = ref<'preview' | 'source'>('preview')

/** The view as a conversation's panel first shows it: no file. */
export function firstFileData(): FileData {
  return { path: '' }
}

/**
 * The view showing `path`, at `line` when one is named, opened for the
 * directory `root` (absent: the conversation's); another line or directory
 * of the file shown moves within it.
 */
export function showFile(data: FileData, path: string, line?: number, root?: string): FileData {
  if (data.path === path && data.line === line && data.root === root) {
    return data
  }
  return {
    path,
    ...(line === undefined ? {} : { line }),
    ...(root === undefined ? {} : { root }),
    // The same file keeps the view it shows in.
    ...(data.path === path && data.view !== undefined ? { view: data.view } : {}),
  }
}

/** The view showing `folder` in its tree, the file shown staying: a move, as to another file. */
export function locateFolder(data: FileData, folder: string): FileData {
  return data.folder === folder ? data : { ...data, folder }
}

/** The view showing its file as `mode` says: a change of how the file is seen, not of the file. */
export function showMode(data: FileData, mode: 'preview' | 'source'): FileData {
  return data.view === mode ? data : { ...data, view: mode }
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

/**
 * The File view's place in the page's address: the file, its directory, its
 * line, the folder its tree went to, and whether it shows the preview or the
 * source, as `?panel=file&path=/work/README.md&view=source`. A file opened
 * without a view is written with the one last chosen (`fileView`), in the
 * same move.
 */
export const filePlace = definePlace({
  schema: z.object({
    path: z.string().optional(),
    root: z.string().optional(),
    line: z.coerce.number<string>().int().positive().optional(),
    view: z.enum(['preview', 'source']).optional(),
    folder: z.string().optional(),
  }),
  write: (data: FileData) => ({
    path: data.path === '' ? undefined : data.path,
    root: data.root,
    line: data.line === undefined ? undefined : String(data.line),
    view: data.path === '' ? undefined : data.view ?? fileView.value,
    folder: data.folder,
  }),
  read: (place, current) => {
    const path = place.path ?? ''
    return {
      path,
      ...(place.root === undefined ? {} : { root: place.root }),
      ...(place.line === undefined ? {} : { line: place.line }),
      ...(place.view === undefined ? {} : { view: place.view }),
      ...(place.folder === undefined ? {} : { folder: place.folder }),
      // Show in Files is asked of the file it was asked for.
      ...(current.tree && current.path === path ? { tree: true as const } : {}),
    }
  },
})
