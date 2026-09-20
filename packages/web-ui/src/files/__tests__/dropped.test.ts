import { expect, test } from 'bun:test'
import { droppedItems, type DroppedDirectory, type DroppedEntry, type DroppedFile } from '../dropped'

function fileEntry(name: string, size: number): DroppedFile {
  return {
    name,
    isDirectory: false,
    file: (success) => success(new File([new Uint8Array(size)], name)),
  }
}

/** A directory whose reader hands its entries over two at a time, as a browser does a hundred at a time. */
function directoryEntry(name: string, children: DroppedEntry[]): DroppedDirectory {
  return {
    name,
    isDirectory: true,
    createReader() {
      let at = 0
      return {
        readEntries(success) {
          const batch = children.slice(at, at + 2)
          at += batch.length
          success(batch)
        },
      }
    },
  }
}

test('a dropped folder reads whole: every folder before what it holds, every file by its path in it', async () => {
  const photos = directoryEntry('photos', [
    fileEntry('a.jpg', 3),
    directoryEntry('2024', [directoryEntry('raw', [fileEntry('b.jpg', 5)]), fileEntry('c.jpg', 1)]),
    directoryEntry('empty', []),
  ])
  const loose = new File(['x'], 'notes.md')
  const items = await droppedItems([photos, fileEntry('logo.svg', 2), loose])
  expect(items.map((item) => item.kind === 'file' ? ['file', item.file.name] : ['folder', item.name])).toEqual([
    ['folder', 'photos'],
    ['file', 'logo.svg'],
    ['file', 'notes.md'],
  ])
  const folder = items[0]!
  if (folder.kind !== 'folder')
    throw new Error('expected a folder')
  expect(folder.directories).toEqual(['2024', '2024/raw', 'empty'])
  expect(folder.files.map((entry) => [entry.path, entry.file.size])).toEqual([
    ['a.jpg', 3],
    ['2024/raw/b.jpg', 5],
    ['2024/c.jpg', 1],
  ])
})

test('a folder that cannot be read fails the drop', async () => {
  const locked: DroppedDirectory = {
    name: 'locked',
    isDirectory: true,
    createReader: () => ({
      readEntries: (_success, failure) => failure?.(new DOMException('Not readable', 'NotReadableError')),
    }),
  }
  await expect(droppedItems([locked])).rejects.toThrow('Not readable')
})
