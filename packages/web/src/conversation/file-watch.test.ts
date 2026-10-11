import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { HostFiles, SUMMARY_REREAD_MS, type KeptSpec } from '@demicodes/web-ui/files/file-cache'
import type { FileWatchMessage, FileWatchRequest } from '../api/generated/web-api'
import { ConversationWatch } from './file-watch'

// A conversation's file watch as the backend plays it (`web-api.md` § File
// watch): what its states cover in the Host's kept files, what its reports
// have read again, and which paths outside the working tree it names.

/** The page's watch socket, which the test plays as the backend would. */
class WatchSocket extends EventTarget {
  readonly sent: FileWatchRequest[] = []
  closed = false

  constructor(readonly url: string) {
    super()
    sockets.push(this)
  }

  send(data: string): void {
    this.sent.push(JSON.parse(data))
  }

  close(): void {
    this.closed = true
  }

  /** The upgrade succeeds. */
  open(): void {
    this.dispatchEvent(new Event('open'))
  }

  receive(message: FileWatchMessage): void {
    this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(message) }))
  }

  /** The backend or the network ends it. */
  end(): void {
    this.closed = true
    this.dispatchEvent(new Event('close'))
  }
}

const sockets: WatchSocket[] = []
const realSocket = globalThis.WebSocket
const realWindow = Reflect.get(globalThis, 'window')

beforeEach(() => {
  sockets.length = 0
  Reflect.set(globalThis, 'WebSocket', WatchSocket)
  if (realWindow === undefined) {
    Reflect.set(globalThis, 'window', { location: { href: 'http://127.0.0.1:3271/chat' } })
  }
})

afterEach(() => {
  Reflect.set(globalThis, 'WebSocket', realSocket)
  Reflect.set(globalThis, 'window', realWindow)
})

/** Lets the reads and the page's queued messages land. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

/** Lets the promises due run, without the clock. */
async function flush(): Promise<void> {
  for (let turn = 0; turn < 10; turn++)
    await Promise.resolve()
}

/** The Host's kept files, each read counted and answered at once with its count. */
function kept() {
  const files = new HostFiles()
  const reads: string[] = []
  const spec = (kind: KeptSpec<string>['kind'], path: string): KeptSpec<string> => ({
    kind,
    path,
    read: async () => {
      reads.push(`${kind} ${path}`)
      return `${reads.length}`
    },
    size: (text) => text.length,
  })
  return { files, reads, spec }
}

/** The working tree's subscription is live. */
const live: FileWatchMessage = { type: 'live', key: 'tree', since: 1, epoch: 'e' }

/** A change of the subscriptions `keys`. */
const changed = (...keys: string[]): FileWatchMessage => ({ type: 'changed', changes: keys.map((key) => [key, 2]) })

test('the watch subscribes to the working tree, whose change reads again what it holds that shows', async () => {
  jest.useFakeTimers()
  try {
    const { files, reads, spec } = kept()
    const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
    watch.show('/w')
    const socket = sockets[0]!
    expect(new URL(socket.url).pathname).toBe('/api/conversations/c1/fs/watch')
    socket.open()
    expect(socket.sent).toEqual([{ type: 'subscribe', subscriptions: [{ key: 'tree', kind: 'worktree', path: '/w' }] }])
    socket.receive(live)
    files.show(spec('changes', '/w'))
    files.show(spec('text', '/w/a.ts'))
    await flush()
    expect(reads).toEqual(['changes /w', 'text /w/a.ts'])

    // Confirmed while live: showing it again asks nothing.
    files.show(spec('text', '/w/a.ts')).release()
    await flush()
    expect(reads).toHaveLength(2)

    // The tree changed: what shows is read again, the list once its second ends.
    socket.receive(changed('tree'))
    jest.advanceTimersByTime(SUMMARY_REREAD_MS)
    await flush()
    expect(reads.slice(2).sort()).toEqual(['changes /w', 'text /w/a.ts'])

    // Its lease is renewed every 3 minutes while the watch is open, its
    // heartbeats keeping it so.
    for (let minute = 0; minute < 3; minute++) {
      jest.advanceTimersByTime(60_000)
      socket.receive({ type: 'heartbeat' })
    }
    expect(socket.closed).toBe(false)
    expect(socket.sent.at(-1)).toEqual({ type: 'renew', keys: ['tree'] })
  } finally {
    jest.useRealTimers()
  }
})

test('a view that shows another file keeps the watch, and nothing shown closes it', async () => {
  const { files, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  const unfollow = watch.show('/w/a.ts')
  const socket = sockets[0]!
  socket.open()
  socket.receive(live)
  const shown = files.show(spec('text', '/w/a.ts'))
  await settle()

  // A view that shows another file lets the last one go first: the watch stays.
  unfollow()
  const other = watch.show('/w/b.ts')
  await settle()
  expect(socket.closed).toBe(false)
  expect(sockets).toHaveLength(1)

  // Nothing shows the files: the watch closes.
  shown.release()
  other()
  await settle()
  expect(socket.closed).toBe(true)
})

test('a path outside the working tree is subscribed to, and covered once it is live', async () => {
  const { files, reads, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  watch.show('/w/a.ts')
  const outside = watch.show('/etc/hosts')
  const socket = sockets[0]!
  socket.open()
  socket.receive(live)
  await settle()
  expect(socket.sent.at(-1)).toEqual({ type: 'subscribe', subscriptions: [{ key: '/etc/hosts', kind: 'file', path: '/etc/hosts' }] })

  // Before it is live, a read there is not confirmed.
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  expect(reads).toEqual(['text /etc/hosts', 'text /etc/hosts'])

  socket.receive({ type: 'live', key: '/etc/hosts', since: 1, epoch: 'e' })
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  expect(reads).toHaveLength(3)
  socket.receive(changed('/etc/hosts'))
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  expect(reads).toHaveLength(4)

  // Shown no more, it is let go and no longer covered.
  outside()
  await settle()
  expect(socket.sent.at(-1)).toEqual({ type: 'unsubscribe', keys: ['/etc/hosts'] })
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  expect(reads).toHaveLength(5)
})

test('a Host that cannot watch and a closed socket leave nothing confirmed', async () => {
  const { files, reads, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  watch.show('/w/a.ts')
  const first = sockets[0]!
  first.open()
  first.receive(live)
  const shown = files.show(spec('text', '/w/a.ts'))
  await settle()

  first.receive({ type: 'unavailable', key: 'tree', reason: 'inotify watch limit reached' })
  expect(watch.note.unavailable).toBe('inotify watch limit reached')
  shown.release()
  files.show(spec('text', '/w/a.ts')).release()
  await settle()
  expect(reads).toHaveLength(2)

  // The socket ends: the watch connects again after its wait, at once when the Host is back.
  first.end()
  expect(watch.note.unavailable).toBeNull()
  watch.online()
  expect(sockets).toHaveLength(2)
})
test('a watch that brings nothing, not even a heartbeat, for 75 s is broken and connects again', async () => {
  jest.useFakeTimers()
  try {
    const { files, reads, spec } = kept()
    const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
    watch.show('/w/a.ts')
    const first = sockets[0]!
    first.open()
    first.receive(live)
    const shown = files.show(spec('text', '/w/a.ts'))
    // The read lands; the clock is the test's, so only promises run.
    await flush()
    // Heartbeats keep a quiet watch.
    jest.advanceTimersByTime(60_000)
    first.receive({ type: 'heartbeat' })
    jest.advanceTimersByTime(74_999)
    expect(first.closed).toBe(false)
    expect(sockets).toHaveLength(1)
    // Silent for 75 s: broken, closed, and what it covered is unconfirmed.
    jest.advanceTimersByTime(1)
    // The judgement waits for the messages already queued.
    jest.advanceTimersByTime(0)
    expect(first.closed).toBe(true)
    shown.release()
    files.show(spec('text', '/w/a.ts')).release()
    await flush()
    expect(reads.filter((read) => read === 'text /w/a.ts').length).toBe(2)
    // The first wait is at most a second.
    jest.advanceTimersByTime(1_000)
    expect(sockets).toHaveLength(2)
  } finally {
    jest.useRealTimers()
  }
})

test('a watch moves to the other path at once, and what it covered waits for the new live', async () => {
  const { files, reads, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  watch.show('/w/a.ts')
  const first = sockets[0]!
  first.open()
  first.receive(live)
  files.show(spec('text', '/w/a.ts')).release()
  await settle()

  // The path to the Host changed: the next connection opens without a wait.
  watch.move()
  expect(first.closed).toBe(true)
  expect(sockets).toHaveLength(2)
  const second = sockets[1]!
  // Until the new watch is live, what was read is checked again.
  files.show(spec('text', '/w/a.ts')).release()
  await settle()
  expect(reads).toEqual(['text /w/a.ts', 'text /w/a.ts'])
  second.open()
  second.receive(live)
  files.show(spec('text', '/w/a.ts')).release()
  await settle()
  files.show(spec('text', '/w/a.ts')).release()
  await settle()
  expect(reads).toHaveLength(3)
})
