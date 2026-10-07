import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { HostFiles, LISTING_REREAD_MS } from '../file-cache'
import { keptSource } from '../kept-source'
import type { FileBrowserEntry } from '../types'

// The page's own writes (`web-application.md` § Requests for one action):
// an upload's entry shows in its folder as the answer says, with no listing
// after it; the Host's watch reports the folder, which is listed once then.
// Cost: no I/O, fake timers; a few milliseconds.

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

/** Lets the Host's answers land: they land in microtasks. */
async function settle(): Promise<void> {
  for (let tick = 0; tick < 10; tick++)
    await Promise.resolve()
}

function host() {
  const lists: string[] = []
  const tree: Record<string, FileBrowserEntry[]> = {
    '/w': [{ name: 'src', isDirectory: true, modifiedAt: '2026-10-01T00:00:00Z' }],
    '/w/src': [{ name: 'app.ts', isDirectory: false, size: 10 }],
  }
  const files = new HostFiles()
  files.cover({ covers: () => true })
  const source = keptSource({
    platform: 'linux',
    home: '/w',
    async list(path) {
      lists.push(path)
      return tree[path] ?? []
    },
    async upload() {},
    async createDirectory() {},
    async remove() {},
  }, { files })
  return { files, lists, source }
}

test('an upload adds its entry to the folder shown, without listing it, until the watch reports the folder', async () => {
  const { files, lists, source } = host()
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

  // The watch's report of the writes lists each folder once, as the
  // second since its last listing ends.
  files.changed(['/w/src/new.ts', '/w/src/app.ts'])
  jest.advanceTimersByTime(LISTING_REREAD_MS)
  await settle()
  expect(lists).toEqual(['/w', '/w/src', '/w/src'])
})
