import { z } from 'zod'

/**
 * What the File view shows (`file-previews.md`): one file of the
 * conversation's Host, by its path, absolute or relative to the workspace,
 * and the files Back and Forward return to.
 */
export const fileDataSchema = z.object({
  /** Empty while it shows no file. */
  path: z.string(),
  /** The line the file was opened at, which the view scrolls to and marks for a moment; absent for none. */
  line: z.number().int().positive().optional(),
  /** The files shown before, latest last. */
  back: z.array(z.string()),
  /** The files left by Back, latest last. */
  forward: z.array(z.string()),
})
export type FileData = z.infer<typeof fileDataSchema>

/** The view as a conversation's panel first shows it: no file, nothing to go back to. */
export function firstFileData(): FileData {
  return { path: '', back: [], forward: [] }
}

/**
 * The view showing `path`, at `line` when one is named, the file it showed
 * kept for Back; another line of the file shown moves within it.
 */
export function showFile(data: FileData, path: string, line?: number): FileData {
  if (data.path === path && data.line === line) {
    return data
  }
  const at = line === undefined ? {} : { line }
  if (data.path === path) {
    return { path, ...at, back: data.back, forward: data.forward }
  }
  return {
    path,
    ...at,
    back: data.path ? [...data.back, data.path] : data.back,
    forward: [],
  }
}

/** The view showing what it showed before, the current file kept for Forward. */
export function goBack(data: FileData): FileData {
  const previous = data.back.at(-1)
  if (previous === undefined) {
    return data
  }
  return { path: previous, back: data.back.slice(0, -1), forward: [...data.forward, data.path] }
}

/** The view showing what Back left, the current file kept for Back. */
export function goForward(data: FileData): FileData {
  const next = data.forward.at(-1)
  if (next === undefined) {
    return data
  }
  return { path: next, forward: data.forward.slice(0, -1), back: [...data.back, data.path] }
}
