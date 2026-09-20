import { expect, test } from 'bun:test'
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

test('uploads go one at a time, in order, and each landing names its directory', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  const changed: string[] = []
  uploads.onChanged((directory) => changed.push(directory))
  uploads.add('/w/assets', added(bytes('a.png', 10), bytes('b.png', 20)))
  expect(source.calls.map((call) => call.path)).toEqual(['/w/assets/a.png'])
  expect(uploads.items.map((item) => item.state.phase)).toEqual(['uploading', 'waiting'])

  // Too soon after the start to tell a rate.
  source.calls[0]!.options.progress(4)
  expect(uploads.items[0]!.state).toEqual({ phase: 'uploading', sent: 4, rate: null })

  source.calls[0]!.done.resolve()
  await settle()
  expect(uploads.items.map((item) => item.state.phase)).toEqual(['done', 'uploading'])
  expect(source.calls.map((call) => call.path)).toEqual(['/w/assets/a.png', '/w/assets/b.png'])
  expect(changed).toEqual(['/w/assets'])
})

test('an upload that overwrites is sent with replace; one that adds is not', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [
    { item: { kind: 'file', file: bytes('a.txt', 1) }, placement: 'overwrite' },
    { item: { kind: 'file', file: bytes('b.txt', 1) }, placement: 'add' },
  ])
  expect(source.calls[0]!.options.replace).toBe(true)
  source.calls[0]!.done.resolve()
  await settle()
  expect(source.calls[1]!.options.replace).toBe(false)
})

test('cancelling the upload on its way aborts it, drops it, and starts the next', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', added(bytes('a', 1), bytes('b', 1)))
  const first = source.calls[0]!
  uploads.cancel(uploads.items[0]!.id)
  expect(first.options.signal.aborted).toBe(true)
  first.done.reject(first.options.signal.reason)
  await settle()
  expect(uploads.items.map((item) => [item.path, item.state.phase])).toEqual([['/w/b', 'uploading']])
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
  expect(uploads.items.map((item) => [item.path, item.state.phase])).toEqual([['/w/b', 'uploading'], ['/w/a', 'waiting']])
  source.calls[1]!.done.resolve()
  await settle()
  source.calls[2]!.done.reject(new Error('Upload connection failed'))
  await settle()
  expect(uploads.items.map((item) => item.state.phase)).toEqual(['done', 'failed'])

  uploads.clearFinished()
  expect(uploads.items).toEqual([])
})

test('an upload on its way tells its pace once it has moved for a moment', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', added(bytes('a', 10_000)))
  await new Promise((resolve) => setTimeout(resolve, 600))
  source.calls[0]!.options.progress(3_000)
  const state = uploads.items[0]!.state
  // About 3,000 bytes in 0.6 seconds, however late the timer fired.
  expect(state.phase === 'uploading' ? state.rate : undefined).toBeGreaterThan(1_500)
  expect(state.phase === 'uploading' ? state.rate : undefined).toBeLessThan(6_000)
})

test('a folder makes itself and the folders in it, then sends its files, counting its bytes across them', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  const changed: string[] = []
  uploads.onChanged((directory) => changed.push(directory))
  uploads.add('/w', [{
    item: folder('photos', ['2024', '2024/raw'], [['a.jpg', 100], ['2024/raw/b.jpg', 50]]),
    placement: 'add',
  }])
  await settle()
  expect(source.log).toEqual(['mkdir /w/photos', 'mkdir /w/photos/2024', 'mkdir /w/photos/2024/raw', 'upload /w/photos/a.jpg'])
  expect(source.calls[0]!.options.replace).toBe(false)
  source.calls[0]!.done.resolve()
  await settle()
  source.calls[1]!.options.progress(20)
  expect(uploads.items[0]!.state).toEqual({ phase: 'uploading', sent: 120, rate: null })
  source.calls[1]!.done.resolve()
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'done' })
  expect(changed).toEqual(['/w', '/w/photos', '/w/photos/2024', '/w/photos', '/w/photos/2024/raw'])
})

test('a file of a folder that fails is noted and the rest go on; a retry sends only what did not land', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [{ item: folder('docs', ['api'], [['a.md', 1], ['api/b.md', 1], ['c.md', 1]]), placement: 'overwrite' }])
  await settle()
  source.calls[0]!.done.resolve()
  await settle()
  source.calls[1]!.done.reject(new FileBrowserError('permission', 'Permission denied'))
  await settle()
  expect(source.calls[1]!.options.replace).toBe(true)
  source.calls[2]!.done.resolve()
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'failed', failures: [{ path: 'api/b.md', message: 'Permission denied' }] })

  source.log.length = 0
  uploads.retry(uploads.items[0]!.id)
  await settle()
  expect(source.log).toEqual(['upload /w/docs/api/b.md'])
  source.calls[3]!.done.resolve()
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'done' })
})

test('replacing deletes what is there first, and a delete that fails stops the folder', async () => {
  const source = fakeSource(new Set(['/w/src/auth']))
  const uploads = new FileUploads(source)
  uploads.add('/w/src', [
    { item: folder('auth', [], [['token.ts', 1]]), placement: 'replace' },
    { item: folder('http', [], [['client.ts', 1]]), placement: 'replace' },
  ])
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'failed', failures: [{ path: '', message: 'Permission denied' }] })
  expect(source.log).toEqual(['remove /w/src/auth', 'remove /w/src/http', 'mkdir /w/src/http', 'upload /w/src/http/client.ts'])
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
