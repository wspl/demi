import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { HostFiles, SUMMARY_REREAD_MS } from '../file-cache'
import { keptChangeSet, keptSource } from '../kept-source'
import type { FileBrowserEntry } from '../types'

// The page's own writes (`web-application.md` § Requests for one action):
// an upload's entry shows in its folder as the answer says, with no listing
// after it; the Host's watch reports the folder, which is listed once then.
// A working directory outside a repository has no changes to list again
// (`file-previews.md` § Changes) until a `.git` appears in it.
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
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  await settle()
  expect(lists).toEqual(['/w', '/w/src', '/w/src'])
})

// A fixed bug: a build writing files in a large repository had the changes
// list read again after every batch of reports, several statuses a second.
test('a changes list that reports keep naming is listed at most once a second, the last report never dropped', async () => {
  const files = new HostFiles()
  files.cover({ covers: () => true })
  let lists = 0
  const changes = keptChangeSet({
    async list() {
      lists += 1
      return { files: [], truncated: false, repository: true, gitDir: '/w/.git', version: null }
    },
    async sides() {
      return { original: '', modified: '', version: null }
    },
  }, '/w', { files })
  changes.show()
  await settle()
  expect(lists).toBe(1)

  for (let step = 0; step < 10; step++) {
    files.changed(['/w/src/gen.cc'])
    await settle()
  }
  expect(lists).toBe(1)
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  await settle()
  expect(lists).toBe(2)
})

test('a changes list outside a repository is not listed again for any change there, until a .git appears', async () => {
  const files = new HostFiles()
  files.cover({ covers: () => true })
  let repository = false
  let lists = 0
  const changes = keptChangeSet({
    async list() {
      lists += 1
      return { files: [], truncated: false, repository, gitDir: repository ? '/w/.git' : null, version: null }
    },
    async sides() {
      return { original: '', modified: '', version: null }
    },
  }, '/w', { files })
  changes.show()
  await settle()
  expect([lists, changes.unavailable]).toEqual([1, 'no-repository'])

  // A process appends to a log there many times a second.
  for (let step = 0; step < 10; step++) {
    files.changed(['/w/app.log'])
    await settle()
  }
  expect(lists).toBe(1)

  // `git init` there: the list is read again, and follows the files from then on.
  repository = true
  files.changed(['/w/.git', '/w/.git/HEAD'])
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  await settle()
  expect([lists, changes.unavailable]).toEqual([2, null])
  files.changed(['/w/app.log'])
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  await settle()
  expect(lists).toBe(3)
})
