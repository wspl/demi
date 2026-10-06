import { expect, jest, test } from 'bun:test'
import { deferred, type Deferred } from '@demicodes/utils'
import { FileUploads, clashPlacement, type UploadItem } from '../file-uploads'
import { FileBrowserError, type FileUploadOptions } from '../types'

/**
 * A source whose file uploads finish when the test says, each call
 * remembered; folders are made and paths deleted at once, unless `refuse`
 * names the path.
 */
function fakeSource(refuse = new Set<string>()) {
  const calls: { path: string; options: FileUploadOptions; done: Deferred<void> }[] = []
  const log: string[] = []
  return {
    calls,
    log,
    upload(path: string, _file: File, options: FileUploadOptions): Promise<void> {
      const done = deferred<void>()
      calls.push({ path, options, done })
      log.push(`upload ${path}`)
      return done.promise
    },
    async createDirectory(path: string): Promise<void> {
      log.push(`mkdir ${path}`)
      if (refuse.has(path))
        throw new FileBrowserError('permission', 'Permission denied')
    },
    async remove(path: string): Promise<void> {
      log.push(`remove ${path}`)
      if (refuse.has(path))
        throw new FileBrowserError('permission', 'Permission denied')
    },
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))
const bytes = (name: string, size: number) => new File([new Uint8Array(size)], name)
const added = (...files: File[]) => files.map((file) => ({ item: { kind: 'file', file } as const, placement: 'add' as const }))

function folder(name: string, directories: string[], files: [string, number][]): UploadItem {
  return {
    kind: 'folder',
    name,
    directories,
    files: files.map(([path, size]) => ({ path, file: bytes(path.split('/').pop()!, size) })),
  }
}

const paths = (calls: { path: string }[]) => calls.map((call) => call.path)

test('up to four steps run at once, taken in the order asked', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w/assets', added(bytes('a.png', 10), bytes('b.png', 20), bytes('c.png', 1), bytes('d.png', 1), bytes('e.png', 1)))
  expect(paths(source.calls)).toEqual(['/w/assets/a.png', '/w/assets/b.png', '/w/assets/c.png', '/w/assets/d.png'])
  expect(uploads.items.map((item) => item.state.phase)).toEqual(['uploading', 'uploading', 'uploading', 'uploading', 'waiting'])

  // Too soon after the start to tell a rate.
  source.calls[0]!.options.progress(4)
  expect(uploads.items[0]!.state).toEqual({ phase: 'uploading', sent: 4, rate: null })

  // The second landing first frees its place for the fifth.
  source.calls[1]!.done.resolve()
  await settle()
  expect(uploads.items.map((item) => item.state.phase)).toEqual(['uploading', 'done', 'uploading', 'uploading', 'uploading'])
  expect(paths(source.calls).at(-1)).toBe('/w/assets/e.png')
})

test('an upload that overwrites is sent with replace; one that adds is not', () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [
    { item: { kind: 'file', file: bytes('a.txt', 1) }, placement: 'overwrite' },
    { item: { kind: 'file', file: bytes('b.txt', 1) }, placement: 'add' },
  ])
  expect(source.calls.map((call) => call.options.replace)).toEqual([true, false])
})

test('cancelling an upload on its way aborts it, drops it, and starts the next in its place', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', added(bytes('a', 1), bytes('b', 1), bytes('c', 1), bytes('d', 1), bytes('e', 1)))
  const first = source.calls[0]!
  uploads.cancel(uploads.items[0]!.id)
  expect(first.options.signal.aborted).toBe(true)
  first.done.reject(first.options.signal.reason)
  await settle()
  expect(uploads.items.map((item) => [item.path, item.state.phase])).toEqual([
    ['/w/b', 'uploading'],
    ['/w/c', 'uploading'],
    ['/w/d', 'uploading'],
    ['/w/e', 'uploading'],
  ])
})

test('a failure stays listed with its reason until retried or cleared', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', added(bytes('a', 1), bytes('b', 1)))
  source.calls[0]!.done.reject(new FileBrowserError('exists'))
  await settle()
  expect(uploads.items[0]!.state).toEqual({
    phase: 'failed',
    failures: [{ path: '', message: 'A file with this name is there now.' }],
  })

  // A retry goes after the ones still waiting or on their way.
  uploads.retry(uploads.items[0]!.id)
  expect(uploads.items.map((item) => [item.path, item.state.phase])).toEqual([['/w/b', 'uploading'], ['/w/a', 'uploading']])
  source.calls[1]!.done.resolve()
  await settle()
  source.calls[2]!.done.reject(new Error('Upload connection failed'))
  await settle()
  expect(uploads.items.map((item) => item.state.phase)).toEqual(['done', 'failed'])

  uploads.clearFinished()
  expect(uploads.items).toEqual([])
})

test('an upload on its way tells its pace once it has moved for a moment', () => {
  jest.useFakeTimers()
  try {
    const source = fakeSource()
    const uploads = new FileUploads(source)
    uploads.add('/w', added(bytes('a', 10_000)))
    jest.advanceTimersByTime(600)
    source.calls[0]!.options.progress(3_000)
    // 3,000 bytes in 0.6 seconds.
    expect(uploads.items[0]!.state).toEqual({ phase: 'uploading', sent: 3_000, rate: 5_000 })
  } finally {
    jest.useRealTimers()
  }
})

test('a folder sends its files at once, whose writes make its folders, makes only its empty folders, and counts its bytes across them', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [{
    item: folder('photos', ['2024', '2024/raw', 'empty'], [['a.jpg', 100], ['2024/raw/b.jpg', 50]]),
    placement: 'add',
  }])
  await settle()
  expect(source.log).toEqual(['upload /w/photos/a.jpg', 'upload /w/photos/2024/raw/b.jpg', 'mkdir /w/photos/empty'])
  expect(source.calls[0]!.options.replace).toBe(false)
  source.calls[0]!.options.progress(30)
  source.calls[1]!.options.progress(20)
  expect(uploads.items[0]!.state).toEqual({ phase: 'uploading', sent: 50, rate: null })
  source.calls[1]!.done.resolve()
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'uploading', sent: 80, rate: null })
  source.calls[0]!.done.resolve()
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'done' })
})

test('an empty folder makes itself', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [{ item: folder('later', [], []), placement: 'add' }])
  await settle()
  expect(source.log).toEqual(['mkdir /w/later'])
  expect(uploads.items[0]!.state).toEqual({ phase: 'done' })
})

test('a file of a folder that fails is noted and the rest go on; a retry sends only what did not land', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [{ item: folder('docs', ['api'], [['a.md', 1], ['api/b.md', 1], ['c.md', 1]]), placement: 'overwrite' }])
  source.calls[0]!.done.resolve()
  source.calls[1]!.done.reject(new FileBrowserError('permission', 'Permission denied'))
  source.calls[2]!.done.resolve()
  await settle()
  expect(source.calls.map((call) => call.options.replace)).toEqual([true, true, true])
  expect(uploads.items[0]!.state).toEqual({ phase: 'failed', failures: [{ path: 'api/b.md', message: 'Permission denied' }] })

  source.log.length = 0
  uploads.retry(uploads.items[0]!.id)
  expect(source.log).toEqual(['upload /w/docs/api/b.md'])
  source.calls[3]!.done.resolve()
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'done' })
})

test('replacing deletes what is there before anything else starts, and a delete that fails stops the folder', async () => {
  const source = fakeSource(new Set(['/w/src/auth']))
  const uploads = new FileUploads(source)
  uploads.add('/w/src', [
    { item: folder('auth', [], [['token.ts', 1]]), placement: 'replace' },
    { item: folder('http', [], [['client.ts', 1]]), placement: 'replace' },
  ])
  expect(source.log).toEqual(['remove /w/src/auth', 'remove /w/src/http'])
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'failed', failures: [{ path: '', message: 'Permission denied' }] })
  expect(source.log).toEqual(['remove /w/src/auth', 'remove /w/src/http', 'upload /w/src/http/client.ts'])
})

test('Replace and Merge place an item by what meets what', () => {
  // A file over a file is written over in place by either answer.
  expect(clashPlacement('replace', false, false)).toBe('overwrite')
  expect(clashPlacement('merge', false, false)).toBe('overwrite')
  // A folder over a folder: Merge goes into it, Replace deletes it first.
  expect(clashPlacement('merge', true, true)).toBe('overwrite')
  expect(clashPlacement('replace', true, true)).toBe('replace')
  // A folder meeting a file, or a file a folder, can only take its place.
  expect(clashPlacement('merge', true, false)).toBe('replace')
  expect(clashPlacement('replace', false, true)).toBe('replace')
  expect(clashPlacement('skip', true, true)).toBeNull()
})
