import type { UploadItem } from './file-uploads'

/**
 * What the walk reads of a dropped entry, the File and Directory Entries
 * API's `FileSystemEntry`: the browser's entries have all of it.
 */
export interface DroppedEntry {
  readonly name: string
  readonly isDirectory: boolean
}

export interface DroppedFile extends DroppedEntry {
  file(success: (file: File) => void, failure?: (error: DOMException) => void): void
}

export interface DroppedDirectory extends DroppedEntry {
  createReader(): {
    readEntries(success: (entries: DroppedEntry[]) => void, failure?: (error: DOMException) => void): void
  }
}

// The API tells an entry's kind by these flags rather than by its class.
function isDirectory(entry: DroppedEntry): entry is DroppedDirectory {
  return entry.isDirectory
}

function isFile(entry: DroppedEntry): entry is DroppedFile {
  return !entry.isDirectory
}

/**
 * The files and folders a drop carries, taken while its event lasts: the
 * transfer empties once the handler returns, while an entry taken from it
 * stays readable. A browser without entries hands over the files alone.
 */
export function droppedEntries(transfer: DataTransfer): (DroppedEntry | File)[] {
  const out: (DroppedEntry | File)[] = []
  for (const item of transfer.items) {
    if (item.kind !== 'file')
      continue
    const entry = item.webkitGetAsEntry()
    const file = entry ? null : item.getAsFile()
    if (entry)
      out.push(entry)
    else if (file)
      out.push(file)
  }
  return out
}

/** Reads what a drop carried into what an upload takes: a file as it is, a folder with everything in it. */
export async function droppedItems(entries: readonly (DroppedEntry | File)[]): Promise<UploadItem[]> {
  const items: UploadItem[] = []
  for (const entry of entries) {
    if (entry instanceof File)
      items.push({ kind: 'file', file: entry })
    else if (isDirectory(entry))
      items.push(await readFolder(entry))
    else if (isFile(entry))
      items.push({ kind: 'file', file: await fileOf(entry) })
  }
  return items
}

/** A dropped folder whole: its folders in the order they are met, each before what it holds, and its files. */
async function readFolder(folder: DroppedDirectory): Promise<UploadItem> {
  const directories: string[] = []
  const files: { path: string; file: File }[] = []
  const walk = async (directory: DroppedDirectory, prefix: string): Promise<void> => {
    for (const entry of await readAll(directory)) {
      const path = prefix ? `${prefix}/${entry.name}` : entry.name
      if (isDirectory(entry)) {
        directories.push(path)
        await walk(entry, path)
      } else if (isFile(entry)) {
        files.push({ path, file: await fileOf(entry) })
      }
    }
  }
  await walk(folder, '')
  return { kind: 'folder', name: folder.name, directories, files }
}

/** Every entry of a directory: a reader hands them over a batch at a time, then an empty one. */
async function readAll(directory: DroppedDirectory): Promise<DroppedEntry[]> {
  const reader = directory.createReader()
  const entries: DroppedEntry[] = []
  for (;;) {
    const batch = await new Promise<DroppedEntry[]>((resolve, reject) => reader.readEntries(resolve, reject))
    if (batch.length === 0)
      return entries
    entries.push(...batch)
  }
}

function fileOf(entry: DroppedFile): Promise<File> {
  return new Promise((resolve, reject) => entry.file(resolve, reject))
}
