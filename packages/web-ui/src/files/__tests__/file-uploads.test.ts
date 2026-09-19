import { expect, test } from 'bun:test'
import { deferred, type Deferred } from '@demicodes/utils'
import { FileUploads } from '../file-uploads'
import { FileBrowserError, type FileUploadOptions } from '../types'

/** A source whose uploads finish when the test says, each call remembered. */
function fakeSource() {
  const calls: { path: string; options: FileUploadOptions; done: Deferred<void> }[] = []
  return {
    calls,
    upload(path: string, _file: File, options: FileUploadOptions): Promise<void> {
      const done = deferred<void>()
      calls.push({ path, options, done })
      return done.promise
    },
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))
const bytes = (name: string, size: number) => new File([new Uint8Array(size)], name)

test('uploads go one at a time, in order, and each landing names its directory', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  const landed: string[] = []
  uploads.onLanded((directory) => landed.push(directory))
  uploads.add('/w/assets', [bytes('a.png', 10), bytes('b.png', 20)], new Set())
  expect(source.calls.map((call) => call.path)).toEqual(['/w/assets/a.png'])
  expect(uploads.items.map((item) => item.state.phase)).toEqual(['uploading', 'waiting'])

  // Too soon after the start to tell a rate.
  source.calls[0]!.options.progress(4)
  expect(uploads.items[0]!.state).toEqual({ phase: 'uploading', sent: 4, rate: null })

  source.calls[0]!.done.resolve()
  await settle()
  expect(uploads.items.map((item) => item.state.phase)).toEqual(['done', 'uploading'])
  expect(source.calls.map((call) => call.path)).toEqual(['/w/assets/a.png', '/w/assets/b.png'])
  expect(landed).toEqual(['/w/assets'])
})

test('a name to replace is sent with replace; the rest are not', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [bytes('a.txt', 1), bytes('b.txt', 1)], new Set(['a.txt']))
  expect(source.calls[0]!.options.replace).toBe(true)
  source.calls[0]!.done.resolve()
  await settle()
  expect(source.calls[1]!.options.replace).toBe(false)
})

test('cancelling the upload on its way aborts it, drops it, and starts the next', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [bytes('a', 1), bytes('b', 1)], new Set())
  const first = source.calls[0]!
  uploads.cancel(uploads.items[0]!.id)
  expect(first.options.signal.aborted).toBe(true)
  first.done.reject(first.options.signal.reason)
  await settle()
  expect(uploads.items.map((item) => [item.file.name, item.state.phase])).toEqual([['b', 'uploading']])
})

test('a failure stays listed with its reason until retried or cleared', async () => {
  const source = fakeSource()
  const uploads = new FileUploads(source)
  uploads.add('/w', [bytes('a', 1), bytes('b', 1)], new Set())
  source.calls[0]!.done.reject(new FileBrowserError('exists'))
  await settle()
  expect(uploads.items[0]!.state).toEqual({ phase: 'failed', message: 'A file with this name is there now.' })

  // A retry goes after the ones still waiting or on their way.
  uploads.retry(uploads.items[0]!.id)
  expect(uploads.items.map((item) => [item.file.name, item.state.phase])).toEqual([['b', 'uploading'], ['a', 'waiting']])
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
  uploads.add('/w', [bytes('a', 10_000)], new Set())
  await new Promise((resolve) => setTimeout(resolve, 600))
  source.calls[0]!.options.progress(3_000)
  const state = uploads.items[0]!.state
  // About 3,000 bytes in 0.6 seconds, however late the timer fired.
  expect(state.phase === 'uploading' ? state.rate : undefined).toBeGreaterThan(1_500)
  expect(state.phase === 'uploading' ? state.rate : undefined).toBeLessThan(6_000)
})
