import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { deferred, type Deferred } from '@demicodes/utils'
import { effectScope } from 'vue'
import { regionRetry } from '../../ui/region-retry'
import { HostFiles, SUMMARY_REREAD_MS, type KeptSpec, type Read, type Showing } from '../file-cache'
import { FileBrowserError, type FileBrowserFailure } from '../types'
import {
  LEASE_MS,
  RENEW_MS,
  WatchChannel,
  type ReadAt,
  type SubscriptionKind,
  type WatchLinkHandlers,
  type WatchMessage,
  type WatchRequest,
} from '../watch-channel'

// What a page keeps of a Host's files (`plugin-pages.md` § What the
// service keeps), played against a scripted Host: its reads answer when the
// test says, each with the Host's number taken when it began, and its watch
// is the page's `WS …/fs/watch` as the test plays the backend. Every entry
// kept holds its subscriptions; whether it is current comes from the
// numbers; only what shows on screen is read.
// Cost: no I/O, fake timers; a few milliseconds each.

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

/** One read of the Host, by its entry, answered when the test says. */
interface Read_ {
  key: string
  readAt: ReadAt
  answer: Deferred<Read<string>>
  answered: boolean
}

const EPOCH = 'run-1'

/** Lets the answers, the watch's batched messages and the Host's replies land. */
async function settle(): Promise<void> {
  for (let tick = 0; tick < 20; tick++)
    await Promise.resolve()
}

/** A Host whose reads and watch the test plays. */
function host(options: { budget?: number; entries?: number } = {}) {
  let number = 0
  const reads: Read_[] = []
  const sent: WatchRequest[] = []
  let handlers: WatchLinkHandlers | null = null
  let connections = 0
  const files = new HostFiles(options.budget, options.entries)
  const watch = new WatchChannel(files, (opened) => {
    handlers = opened
    connections += 1
    return { send: (request) => sent.push(request), close: () => {} }
  })
  function spec(kind: KeptSpec<string>['kind'], path: string, extra: Partial<KeptSpec<string>> = {}): KeptSpec<string> {
    return {
      kind,
      path,
      watch,
      read() {
        const answer = deferred<Read<string>>()
        reads.push({ key: `${kind} ${path}`, readAt: { epoch: EPOCH, number }, answer, answered: false })
        return answer.promise
      },
      size: (text) => text.length,
      ...extra,
    }
  }
  /** Answers every read on its way with `text`, each with its own number. */
  async function answer(text: string): Promise<void> {
    for (const read of reads.filter((read) => !read.answered)) {
      read.answer.resolve({ value: text, readAt: read.readAt })
      read.answered = true
    }
    await settle()
  }
  /** Answers every read on its way with the Host's refusal that is an answer, such as a file that is not text. */
  async function refuse(kind: FileBrowserFailure['kind']): Promise<void> {
    for (const read of reads.filter((read) => !read.answered)) {
      read.answer.reject(new FileBrowserError(kind, undefined, { version: 'v1', readAt: read.readAt }))
      read.answered = true
    }
    await settle()
  }
  /** The watch's connection opens, as the backend accepts it. */
  async function open(): Promise<void> {
    await settle()
    handlers!.opened()
    await settle()
  }
  /** The key the page subscribed `kind` of `path` under, last. */
  function keyOf(kind: SubscriptionKind, path: string): string {
    for (const request of [...sent].reverse()) {
      if (request.type !== 'subscribe')
        continue
      const found = request.subscriptions.find((subscription) => subscription.kind === kind && subscription.path === path)
      if (found)
        return found.key
    }
    throw new Error(`No subscription to ${kind} ${path}`)
  }
  async function tell(message: WatchMessage): Promise<void> {
    handlers!.message(message)
    await settle()
  }
  /** The Host watches `kind` of `path` from its number now. */
  const live = (kind: SubscriptionKind, path: string) =>
    tell({ type: 'live', key: keyOf(kind, path), since: number, epoch: EPOCH })
  /** A file system event: the Host's number moves on, and `changes` say which subscriptions it changed. */
  async function event(...changes: [SubscriptionKind, string][]): Promise<number> {
    number += 1
    if (changes.length > 0)
      await tell({ type: 'changed', changes: changes.map(([kind, path]) => [keyOf(kind, path), number]) })
    return number
  }
  /** Time passes, the backend's heartbeat keeping the quiet watch alive. */
  async function wait(ms: number): Promise<void> {
    for (let left = ms; left > 0; left -= 30_000) {
      jest.advanceTimersByTime(Math.min(left, 30_000))
      if (handlers)
        await tell({ type: 'heartbeat' })
    }
  }
  const readAt = (): ReadAt => ({ epoch: EPOCH, number })
  const asked = () => reads.map((read) => read.key)
  const requests = (type: WatchRequest['type']) => sent.filter((request) => request.type === type)
  return {
    files, watch, spec, answer, refuse, open, keyOf, live, event, tell, wait, readAt, asked, requests, sent,
    get connections() {
      return connections
    },
  }
}

test('an entry read under its live subscription shows again without a request until a change names it', async () => {
  const h = host()
  const first = h.files.show(h.spec('text', '/w/src/app.ts'))
  await h.open()
  await h.live('file', '/w/src/app.ts')
  await h.answer('one')
  first.release()

  const again = h.files.show(h.spec('text', '/w/src/app.ts'))
  expect(again.entry).toMatchObject({ value: 'one', answer: 'value', reading: false })
  expect(await h.files.get(h.spec('text', '/w/src/app.ts'))).toBe('one')
  expect(h.asked()).toEqual(['text /w/src/app.ts'])

  await h.event(['file', '/w/src/app.ts'])
  expect(h.asked()).toEqual(['text /w/src/app.ts', 'text /w/src/app.ts'])
  // In place: the old text shows while the new one is read.
  expect(again.entry).toMatchObject({ value: 'one', reading: true })
  await h.answer('two')
  expect(again.entry.value).toBe('two')
})

test('numbers decide whether an answer holds, whichever of the read and the change reaches the page first', async () => {
  const h = host()
  const shown = h.files.show(h.spec('text', '/w/a.ts'))
  await h.open()
  await h.live('file', '/w/a.ts')
  await h.answer('read at 0')

  // A change at 1, then a read that began at 1: the answer holds the change.
  await h.event(['file', '/w/a.ts'])
  expect(h.asked()).toHaveLength(2)
  await h.answer('read at 1')
  expect(h.asked()).toHaveLength(2)

  // A read that began at 1 and a change at 2 that reaches the page before
  // its answer: the answer is out of date when it lands, and read once more.
  shown.retry()
  expect(h.asked()).toHaveLength(3)
  await h.event(['file', '/w/a.ts'])
  expect(h.asked()).toHaveLength(3)
  await h.answer('read at 1')
  expect(h.asked()).toHaveLength(4)
  await h.answer('read at 2')
  expect(h.asked()).toHaveLength(4)
  expect(shown.entry.value).toBe('read at 2')

  // An event the Host reported only after a read that began past it: the
  // answer already holds it, and nothing is read for it.
  const late = await h.event()
  shown.retry()
  await h.answer('read at 3')
  await h.tell({ type: 'changed', changes: [[h.keyOf('file', '/w/a.ts'), late]] })
  expect(h.asked()).toHaveLength(5)
})

test('a change of an entry off screen is only noted, and it is read once when it comes back', async () => {
  const h = host()
  const shown = h.files.show(h.spec('text', '/w/a.ts'))
  await h.open()
  await h.live('file', '/w/a.ts')
  await h.answer('one')
  shown.away()

  await h.event(['file', '/w/a.ts'])
  await h.event(['file', '/w/a.ts'])
  expect(h.asked()).toEqual(['text /w/a.ts'])

  shown.back()
  expect(shown.entry).toMatchObject({ value: 'one', reading: true })
  await h.answer('three')
  expect(shown.entry.value).toBe('three')
  shown.away()
  shown.back()
  expect(h.asked()).toEqual(['text /w/a.ts', 'text /w/a.ts'])
})

test('a file that is not text is an answer: kept, current like a text, and its card stays while it is checked', async () => {
  const h = host()
  const shown = h.files.show(h.spec('text', '/w/logo.bin'))
  await h.open()
  await h.live('file', '/w/logo.bin')
  await h.refuse('binary')
  expect(shown.entry).toMatchObject({ answer: 'binary', failure: null })
  shown.release()

  // Shown again, it is current: nothing is asked.
  const again = h.files.show(h.spec('text', '/w/logo.bin'))
  expect(h.asked()).toHaveLength(1)

  // A change reads it again; what shows stays the card until the answer.
  await h.event(['file', '/w/logo.bin'])
  expect(again.entry).toMatchObject({ answer: 'binary', reading: true, failure: null })
  await h.refuse('binary')
  expect(again.entry).toMatchObject({ answer: 'binary', reading: false })

  // Only a read that could not be made is a failure, beside what stays shown.
  await h.event(['file', '/w/logo.bin'])
  await h.refuse('offline')
  expect(again.entry).toMatchObject({ answer: 'binary', failure: { kind: 'offline' } })
})

test('a listing and a changes list are read at most once a second, the last change never dropped', async () => {
  const h = host()
  h.files.show(h.spec('listing', '/w/logs'))
  h.files.show(h.spec('text', '/w/logs/app.log'))
  await h.open()
  await h.live('folder_names', '/w/logs')
  await h.live('file', '/w/logs/app.log')
  await h.answer('0')
  jest.advanceTimersByTime(5000)
  const listingReads = () => h.asked().filter((key) => key === 'listing /w/logs').length
  const textReads = () => h.asked().filter((key) => key === 'text /w/logs/app.log').length

  // A log appended every 100 ms for 3 s, a new file among its first writes.
  for (let step = 1; step <= 30; step++) {
    jest.advanceTimersByTime(100)
    await h.event(['file', '/w/logs/app.log'], ...(step === 1 ? [['folder_names', '/w/logs'] as [SubscriptionKind, string]] : []))
    await h.answer(String(step))
  }
  expect(textReads()).toBe(31)
  expect(listingReads()).toBe(2)

  // A change a second or more after the last read is read at once; those
  // within the next second wait for it to end, and are read then, once.
  await h.event(['folder_names', '/w/logs'])
  expect(listingReads()).toBe(3)
  await h.answer('listed')
  await h.event(['folder_names', '/w/logs'])
  await h.event(['folder_names', '/w/logs'])
  expect(listingReads()).toBe(3)
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  expect(listingReads()).toBe(4)
})

test('a folder the tree and a folder dialog both keep follows the details, and the names again once only the tree holds it', async () => {
  const h = host()
  const tree = h.files.show(h.spec('listing', '/w/src'))
  await h.open()
  expect(h.requests('subscribe')).toEqual([{ type: 'subscribe', subscriptions: [{ key: expect.any(String), kind: 'folder_names', path: '/w/src' }] }])

  const dialog = h.files.show(h.spec('listing', '/w/src', { details: true }))
  await h.answer('entries')
  const names = h.keyOf('folder_names', '/w/src')
  expect(h.keyOf('folder_details', '/w/src')).not.toBe(names)
  expect(h.requests('unsubscribe')).toEqual([{ type: 'unsubscribe', keys: [names] }])

  dialog.release()
  await settle()
  expect(h.keyOf('folder_names', '/w/src')).not.toBe(names)
  tree.release()
})

test('leases run from when the page sent what the Host acknowledged, so the page counts one over first, and subscribes again', async () => {
  const h = host()
  const shown = h.files.show(h.spec('text', '/w/a.ts'))
  await h.open()
  const key = h.keyOf('file', '/w/a.ts')
  await h.live('file', '/w/a.ts')
  await h.answer('one')

  // Every 3 minutes one renew names every subscription held; the Host
  // acknowledges the first a second after it went, and no later one.
  await h.wait(RENEW_MS)
  expect(h.requests('renew')).toEqual([{ type: 'renew', keys: [key] }])
  const renewedFrom = Date.now()
  await h.wait(1000)
  await h.tell({ type: 'renewed', keys: [key] })
  shown.release()

  // The backend counts that lease from when the renew arrived; the page
  // from when it sent it: at that moment plus 10 minutes the page no longer
  // takes the answer as current, and reads it when it is shown.
  jest.setSystemTime(renewedFrom + LEASE_MS - 1)
  const before = h.files.show(h.spec('text', '/w/a.ts'))
  expect(h.asked()).toHaveLength(1)
  before.release()
  jest.setSystemTime(renewedFrom + LEASE_MS)
  const after = h.files.show(h.spec('text', '/w/a.ts'))
  expect(after.entry).toMatchObject({ value: 'one', reading: true })
  expect(h.asked()).toHaveLength(2)
  // It is subscribed again under its key, at once.
  await settle()
  expect(h.requests('subscribe').at(-1)).toEqual({ type: 'subscribe', subscriptions: [{ key, kind: 'file', path: '/w/a.ts' }] })
})

test('an entry whose lease lapsed off screen is subscribed again at the next renew and read once when it shows', async () => {
  const h = host()
  const shown = h.files.show(h.spec('text', '/w/a.ts'))
  await h.open()
  const key = h.keyOf('file', '/w/a.ts')
  await h.live('file', '/w/a.ts')
  await h.answer('one')
  shown.release()

  // No renew is acknowledged: the lease lapses as the page counts it, and
  // the renew after that subscribes it again instead of renewing it.
  await h.wait(4 * RENEW_MS)
  expect(h.requests('subscribe')).toHaveLength(2)
  expect(h.requests('subscribe').at(-1)).toEqual({ type: 'subscribe', subscriptions: [{ key, kind: 'file', path: '/w/a.ts' }] })
  await h.event()
  await h.live('file', '/w/a.ts')
  expect(h.asked()).toHaveLength(1)

  const again = h.files.show(h.spec('text', '/w/a.ts'))
  expect(again.entry).toMatchObject({ value: 'one', reading: true })
  await h.answer('one, checked')
  expect(h.asked()).toHaveLength(2)
  again.release()
  h.files.show(h.spec('text', '/w/a.ts'))
  expect(h.asked()).toHaveLength(2)
})

test('an entry that leaves the cache is unsubscribed at once, a subject other entries hold is kept', async () => {
  const h = host({ entries: 2 })
  const a = h.files.show(h.spec('text', '/w/a.ts'))
  const sides = h.files.show(h.spec('sides', '/w/b.ts', { root: '/w' }))
  await h.open()
  await h.answer('x')
  const fileA = h.keyOf('file', '/w/a.ts')
  a.release()
  sides.release()

  // A third entry is past the count: the one shown longest ago goes, and
  // its subscription with it, in the same turn, before any timer.
  h.files.show(h.spec('committed', '/w/c.ts', { root: '/w' }))
  await settle()
  expect(h.requests('unsubscribe')).toEqual([{ type: 'unsubscribe', keys: [fileA] }])
  // The `head` the sides held is the committed contents' too, so it stays.
  expect(h.sent.filter((request) => request.type === 'subscribe').flatMap((request) => request.subscriptions.map((subscription) => subscription.kind)))
    .toEqual(['file', 'file', 'head'])
})

test('past the text budget the entries shown longest ago go first, and none a view holds', async () => {
  const h = host({ budget: 10 })
  const kept = h.files.show(h.spec('text', '/w/kept.ts'))
  const old = h.files.show(h.spec('text', '/w/old.ts'))
  await h.answer('12345')
  old.release()
  const recent = h.files.show(h.spec('text', '/w/recent.ts'))
  await h.answer('12345')

  expect(kept.entry.value).toBe('12345')
  expect(recent.entry.value).toBe('12345')
  expect(h.files.peek(h.spec('text', '/w/old.ts')).value).toBeUndefined()
})

test('a Host that cannot watch a subscription leaves its entry out of date, said quietly and checked each time it shows', async () => {
  const h = host()
  const shown = h.files.show(h.spec('text', '/far/a.log'))
  const other = h.files.show(h.spec('text', '/w/b.ts'))
  await h.open()
  await h.live('file', '/w/b.ts')
  await h.tell({ type: 'unavailable', key: h.keyOf('file', '/far/a.log'), reason: 'Too many watches.' })
  await h.answer('x')
  expect(shown.entry.unwatched).toBe('Too many watches.')
  expect(other.entry.unwatched).toBeNull()
  shown.release()
  other.release()

  h.files.show(h.spec('text', '/far/a.log'))
  h.files.show(h.spec('text', '/w/b.ts'))
  expect(h.asked()).toEqual(['text /far/a.log', 'text /w/b.ts', 'text /far/a.log'])
})

test('a watch that reconnects subscribes again to everything kept, and what shows is checked once the numbers say', async () => {
  const h = host()
  const shown = h.files.show(h.spec('text', '/w/a.ts'))
  h.files.show(h.spec('text', '/w/b.ts')).away()
  await h.open()
  await h.live('file', '/w/a.ts')
  await h.live('file', '/w/b.ts')
  await h.answer('one')
  expect(h.asked()).toHaveLength(2)

  // A new run of the runner: its numbers start over.
  await h.tell({ type: 'offline' })
  await h.open()
  expect(h.requests('subscribe').at(-1)).toMatchObject({ subscriptions: [{ path: '/w/a.ts' }, { path: '/w/b.ts' }] })
  await h.tell({ type: 'live', key: h.keyOf('file', '/w/a.ts'), since: 0, epoch: 'run-2' })
  await h.tell({ type: 'live', key: h.keyOf('file', '/w/b.ts'), since: 0, epoch: 'run-2' })
  // The shown one is read again in the background; the one away is not.
  expect(h.asked()).toEqual(['text /w/a.ts', 'text /w/b.ts', 'text /w/a.ts'])
  expect(shown.entry).toMatchObject({ value: 'one', reading: true })
})

test('a change reads a shared entry through the route of the view that came on screen last', async () => {
  const h = host()
  const routes: string[] = []
  const via = (route: string): Partial<KeptSpec<string>> => ({
    read: async () => {
      routes.push(route)
      return { value: route, readAt: h.readAt() }
    },
  })
  h.files.show(h.spec('text', '/w/a.ts', via('conversation')))
  await h.open()
  await h.live('file', '/w/a.ts')
  await settle()
  const dialog = h.files.show(h.spec('text', '/w/a.ts', via('device')))
  expect(routes).toEqual(['conversation'])

  await h.event(['file', '/w/a.ts'])
  dialog.away()
  await h.event(['file', '/w/a.ts'])
  expect(routes).toEqual(['conversation', 'device', 'conversation'])
})

test('a Retry ends when its read does, though the file is still not there or the Host still out of reach', async () => {
  const h = host()
  const scope = effectScope()
  // What a view's Retry runs: its region loads from the click until the read ends (`RegionStatus`).
  const retried = (shown: Showing<string>) => {
    const region = scope.run(() => regionRetry(() => [shown.entry.answer, shown.entry.failure?.kind]))!
    region.retry(() => shown.retry())
    return region.retrying
  }

  // `~` typed as a file of the workspace: nothing is there, and nothing is there again.
  const missing = h.files.show(h.spec('text', '/w/~'))
  await h.refuse('not-found')
  expect(missing.entry).toMatchObject({ answer: 'gone', failure: null })
  const loading = retried(missing)
  expect(loading.value).toBe(true)
  await h.refuse('not-found')
  expect(loading.value).toBe(false)
  expect(missing.entry).toMatchObject({ answer: 'gone', reading: false })

  // A read that could not be made, then could not be made again.
  const offline = h.files.show(h.spec('text', '/w/a.ts'))
  await h.refuse('offline')
  const again = retried(offline)
  expect(again.value).toBe(true)
  await h.refuse('offline')
  expect(again.value).toBe(false)
  expect(offline.entry).toMatchObject({ answer: null, failure: { kind: 'offline' }, reading: false })
  scope.stop()
})
