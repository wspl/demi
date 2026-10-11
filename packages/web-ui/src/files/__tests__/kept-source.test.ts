import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { HostFiles, SUMMARY_REREAD_MS } from '../file-cache'
import { keptChangeSet, keptSource } from '../kept-source'
import { SimulatedWatch } from '../memory-source'
import type { FileBrowserEntry } from '../types'
import { WatchChannel } from '../watch-channel'

// The page's own writes (`web-application.md` § Requests for one action):
// an upload's entry shows in its folder as the answer says, with no listing
// after it; the Host's watch reports the folder, which is listed once then.
// And a changes list a build keeps changing is listed at most once a
// second. Played against a simulated Host's watch and numbers.
// Cost: no I/O, fake timers; a few milliseconds.

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

/** Lets the Host's answers and its watch's messages land: they land in microtasks. */
async function settle(): Promise<void> {
  for (let tick = 0; tick < 20; tick++)
    await Promise.resolve()
}

function host() {
  const lists: string[] = []
  const tree: Record<string, FileBrowserEntry[]> = {
    '/w': [{ name: 'src', isDirectory: true, modifiedAt: '2026-10-01T00:00:00Z' }],
    '/w/src': [{ name: 'app.ts', isDirectory: false, size: 10 }],
  }
  const files = new HostFiles()
  const watched = new SimulatedWatch()
  const watch = new WatchChannel(files, watched.open)
  const source = keptSource({
    platform: 'linux',
    home: '/w',
    async list(path) {
      lists.push(path)
      return { value: tree[path] ?? [], readAt: watched.readAt() }
    },
    async upload() {},
    async createDirectory() {},
    async remove() {},
  }, { files, watch })
  return { files, watch, watched, lists, source }
}

test('an upload adds its entry to the folder shown, without listing it, until the watch reports the folder', async () => {
  const { watched, lists, source } = host()
  const root = source.showListing('/w')
  const src = source.showListing('/w/src')
  await settle()
  expect(lists).toEqual(['/w', '/w/src'])

  const options = { replace: false, signal: new AbortController().signal, progress: () => {} }
  await source.upload!('/w/src/new.ts', new File(['12345'], 'new.ts'), options)
  await source.upload!('/w/assets/logo.png', new File(['1'], 'logo.png'), options)
  expect(src.entry.value?.map((entry) => [entry.name, entry.size])).toEqual([['app.ts', 10], ['new.ts', 5]])
  // The folder the second write made shows in its own folder; one listed already keeps what was listed.
  expect(root.entry.value).toEqual([
    { name: 'src', isDirectory: true, modifiedAt: '2026-10-01T00:00:00Z' },
    { name: 'assets', isDirectory: true },
  ])
  await source.remove!('/w/src/app.ts')
  expect(src.entry.value?.map((entry) => entry.name)).toEqual(['new.ts'])
  await settle()
  expect(lists).toEqual(['/w', '/w/src'])

  // The watch's report of the writes lists the folder once, as the second
  // since its last listing ends.
  watched.change('/w/src/new.ts', { entry: true })
  watched.change('/w/src/app.ts', { entry: true })
  await settle()
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  await settle()
  expect(lists).toEqual(['/w', '/w/src', '/w/src'])
})

// A fixed bug: a build writing files in a large repository had the changes
// list read again after every batch of reports, several statuses a second.
test('a changes list that changes keep naming is listed at most once a second, the last change never dropped', async () => {
  const files = new HostFiles()
  const watched = new SimulatedWatch()
  let lists = 0
  const set = keptChangeSet({
    async list() {
      lists += 1
      return { value: { files: [], truncated: false, repository: true, version: null }, readAt: watched.readAt() }
    },
    async sides() {
      return { value: { original: '', modified: '', version: null }, readAt: watched.readAt() }
    },
  }, '/w', { files, watch: new WatchChannel(files, watched.open) })
  set.show()
  await settle()
  expect(lists).toBe(1)
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)

  // A build writes for 2.5 s, a batch every 100 ms: the list is read at
  // the first batch and as each second ends, and once more after the last.
  for (let batch = 0; batch < 25; batch++) {
    watched.change(`/w/out/${batch}.o`)
    await settle()
    jest.advanceTimersByTime(100)
  }
  await settle()
  expect(lists).toBe(4)
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  await settle()
  expect(lists).toBe(5)
  jest.advanceTimersByTime(5 * SUMMARY_REREAD_MS)
  await settle()
  expect(lists).toBe(5)
})
