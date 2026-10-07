import { expect, test } from 'bun:test'
import { deferred, type Deferred } from '@demicodes/utils'
import { HostFiles, type Coverage, type KeptSpec } from '../file-cache'
import { FileBrowserError } from '../types'

// What a page keeps of a Host's files (`plugin-pages.md` § What the service
// keeps): an entry read under a live watch shows again without a request
// until a report names it; anything else shows at once and is checked.

/** One read of the Host, by its entry, answered when the test says. */
interface Read {
  key: string
  answer: Deferred<string>
  answered: boolean
}

/** A Host whose reads answer when the test says, each counted by its entry. */
function host() {
  const reads: Read[] = []
  function spec(kind: KeptSpec<string>['kind'], path: string): KeptSpec<string> {
    return {
      kind,
      path,
      read() {
        const answer = deferred<string>()
        reads.push({ key: `${kind} ${path}`, answer, answered: false })
        return answer.promise
      },
      size: (text) => text.length,
    }
  }
  /** Answers every read on its way with `text`, and lets the answers land. */
  async function answer(text: string): Promise<void> {
    for (const read of reads.filter((read) => !read.answered)) {
      read.answer.resolve(text)
      read.answered = true
    }
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
  const asked = () => reads.map((read) => read.key)
  return { reads, spec, answer, asked }
}

const everything: Coverage = { covers: () => true }

test('an entry read under a live watch shows again at once without a request', async () => {
  const files = new HostFiles()
  files.cover(everything)
  const { spec, answer, asked } = host()
  const first = files.show(spec('text', '/w/src/app.ts'))
  expect(first.entry).toMatchObject({ value: undefined, reading: true })
  await answer('one')
  first.release()

  const again = files.show(spec('text', '/w/src/app.ts'))
  expect(again.entry).toMatchObject({ value: 'one', reading: false })
  expect(await files.get(spec('text', '/w/src/app.ts'))).toBe('one')
  expect(asked()).toEqual(['text /w/src/app.ts'])
})

test('a report unconfirms the file, its folder and the changes list, and reads again only what shows', async () => {
  const files = new HostFiles()
  files.cover(everything)
  const { spec, answer, asked } = host()
  const text = files.show(spec('text', '/w/src/app.ts'))
  const listing = files.show(spec('listing', '/w/src'))
  const changes = files.show(spec('changes', '/w'))
  const other = files.show(spec('text', '/w/README.md'))
  await answer('before')
  // The listing and the changes list are not shown any more; they are checked when shown next.
  listing.release()
  changes.release()

  files.changed(['/w/src/app.ts'])
  expect(asked().slice(4)).toEqual(['text /w/src/app.ts'])
  // In place: the old text shows while the new one is read.
  expect(text.entry).toMatchObject({ value: 'before', reading: true })
  await answer('after')
  expect(text.entry.value).toBe('after')
  expect(other.entry.reading).toBe(false)

  const listingAgain = files.show(spec('listing', '/w/src'))
  const changesAgain = files.show(spec('changes', '/w'))
  expect(listingAgain.entry).toMatchObject({ value: 'before', reading: true })
  expect(changesAgain.entry).toMatchObject({ value: 'before', reading: true })
  files.show(spec('text', '/w/README.md'))
  expect(asked().slice(5)).toEqual(['listing /w/src', 'changes /w'])
})

test('a change under .git unconfirms the changes list and the committed sides', async () => {
  const files = new HostFiles()
  files.cover(everything)
  const { spec, answer, asked } = host()
  files.show(spec('changes', '/w'))
  files.show(spec('sides', '/w/src/app.ts'))
  files.show(spec('text', '/w/src/app.ts'))
  await answer('x')
  files.changed(['/w/.git/index'])
  expect(asked().slice(3)).toEqual(['changes /w', 'sides /w/src/app.ts'])
})

test('a watch that lost reports leaves nothing it covered confirmed, and what shows is read again', async () => {
  const files = new HostFiles()
  files.cover(everything)
  const { spec, answer, asked } = host()
  const shown = files.show(spec('text', '/w/a.ts'))
  files.show(spec('listing', '/w')).release()
  await answer('x')
  files.uncover(everything, true)
  expect(asked().slice(2)).toEqual(['text /w/a.ts'])
  expect(shown.entry.value).toBe('x')
  await answer('y')
  // Read while nothing covered it: shown again, it is checked.
  shown.release()
  files.show(spec('text', '/w/a.ts'))
  files.show(spec('listing', '/w'))
  expect(asked().slice(3)).toEqual(['text /w/a.ts', 'listing /w'])
})

test('an unconfirmed entry shows at once and is checked once, however many show it meanwhile', async () => {
  const files = new HostFiles()
  const { spec, answer, asked } = host()
  files.show(spec('text', '/w/a.ts')).release()
  await answer('old')
  const first = files.show(spec('text', '/w/a.ts'))
  const second = files.show(spec('text', '/w/a.ts'))
  const once = files.get(spec('text', '/w/a.ts'))
  expect(first.entry).toMatchObject({ value: 'old', reading: true })
  expect(asked()).toEqual(['text /w/a.ts', 'text /w/a.ts'])
  await answer('new')
  expect(await once).toBe('new')
  expect(second.entry.value).toBe('new')
})

test('a read that fails keeps what shows and says why; one that finds the path gone shows that', async () => {
  const files = new HostFiles()
  const { reads, spec, answer } = host()
  const shown = files.show(spec('text', '/w/a.ts'))
  await answer('kept')
  shown.retry()
  const failing = reads.at(-1)!
  failing.answer.reject(new Error('Host unreachable'))
  failing.answered = true
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(shown.entry).toMatchObject({ value: 'kept', failure: { kind: 'other', message: 'Host unreachable' } })

  shown.retry()
  const gone = reads.at(-1)!
  gone.answer.reject(new FileBrowserError('not-found', 'No such file'))
  gone.answered = true
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(shown.entry).toMatchObject({ value: undefined, failure: { kind: 'not-found' } })
})

test('a Retry of a read that failed with nothing to show reads again without the failure, at once', async () => {
  const files = new HostFiles()
  const { reads, spec } = host()
  const shown = files.show(spec('text', '/w/a.ts'))
  const failing = reads.at(-1)!
  failing.answer.reject(new Error('Host unreachable'))
  failing.answered = true
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(shown.entry).toMatchObject({ value: undefined, failure: { kind: 'other' } })
  shown.retry()
  // The view shows the read, not the failure, while it runs (`RegionStatus`).
  expect(shown.entry).toMatchObject({ value: undefined, failure: null, reading: true })
})

test('a read that failed while the Host was away is read again once a watch is live, and nothing else is', async () => {
  const files = new HostFiles()
  const { reads, spec, answer, asked } = host()
  const shown = files.show(spec('text', '/w/a.ts'))
  const failing = reads.at(-1)!
  failing.answer.reject(new FileBrowserError('offline', 'The device is offline.'))
  failing.answered = true
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(shown.entry).toMatchObject({ value: undefined, failure: { kind: 'offline' } })
  // A file confirmed under another live watch stays as it is.
  files.cover({ covers: (path) => path === '/w/b.ts' })
  files.show(spec('text', '/w/b.ts'))
  await answer('b')
  // The Host is back: its watch goes live, and the failed read is made again.
  files.cover(everything)
  expect(asked()).toEqual(['text /w/a.ts', 'text /w/b.ts', 'text /w/a.ts'])
  await answer('back')
  expect(shown.entry).toMatchObject({ value: 'back', failure: null })
})

test('past the budget the entries shown longest ago go first, and none that shows', async () => {
  const files = new HostFiles(10)
  files.cover(everything)
  const { spec, answer, asked } = host()
  files.show(spec('text', '/w/a')).release()
  await answer('aaaa')
  const b = files.show(spec('text', '/w/b'))
  await answer('bbbb')
  files.show(spec('text', '/w/c')).release()
  await answer('cccc')
  // a went: 12 characters were over 10. b shows, so it stays; c is newer.
  files.show(spec('text', '/w/a'))
  files.show(spec('text', '/w/c'))
  expect(b.entry.value).toBe('bbbb')
  expect(asked().slice(3)).toEqual(['text /w/a'])
})
