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

const live: FileWatchMessage = { type: 'state', state: 'live' }

test('a path git ignores reads its file again but leaves the changes list, which a tracked path or .git reads again', async () => {
  jest.useFakeTimers()
  try {
    await ignoredPathsLeaveTheList()
  } finally {
    jest.useRealTimers()
  }
})

async function ignoredPathsLeaveTheList(): Promise<void> {
  const { files, reads, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  watch.show('/w')
  const socket = sockets[0]!
  socket.open()
  socket.receive(live)
  files.show(spec('changes', '/w'))
  files.show(spec('text', '/w/server.log'))
  await flush()

  // A process appends to a log git ignores: the File view follows it, the Change list asks nothing.
  socket.receive({ type: 'changed', paths: ['/w/server.log'], entries: ['/w/server.log'], ignored: ['/w/server.log'] })
  await flush()
  expect(reads.slice(2)).toEqual(['text /w/server.log'])

  // A tracked path reads the list again, once the second since its last read ends.
  socket.receive({ type: 'changed', paths: ['/w/app.ts', '/w/server.log'], entries: ['/w/app.ts', '/w/server.log'], ignored: ['/w/server.log'] })
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  await flush()
  expect(reads.slice(3)).toEqual(['text /w/server.log', 'changes /w'])

  socket.receive({ type: 'changed', paths: ['/w/.git/index'], entries: ['/w/.git/index'], ignored: [] })
  jest.advanceTimersByTime(SUMMARY_REREAD_MS)
  await flush()
  expect(reads.slice(5)).toEqual(['changes /w'])
}

test('a live watch confirms what is read, and a report has what it names read again', async () => {
  const { files, reads, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  const unfollow = watch.show('/w/a.ts')
  const socket = sockets[0]!
  expect(new URL(socket.url).pathname).toBe('/api/conversations/c1/fs/watch')
  socket.open()
  socket.receive(live)

  const shown = files.show(spec('text', '/w/a.ts'))
  await settle()
  shown.release()
  files.show(spec('text', '/w/a.ts')).release()
  expect(reads).toEqual(['text /w/a.ts'])

  const again = files.show(spec('text', '/w/a.ts'))
  socket.receive({ type: 'changed', paths: ['/w/a.ts'], entries: ['/w/a.ts'], ignored: [] })
  await settle()
  expect(reads).toEqual(['text /w/a.ts', 'text /w/a.ts'])
  expect(again.entry.value).toBe('2')

  // A view that shows another file lets the last one go first: the watch stays.
  unfollow()
  const other = watch.show('/w/b.ts')
  await settle()
  expect(socket.closed).toBe(false)
  expect(sockets).toHaveLength(1)

  // Nothing shows the files: the watch closes.
  again.release()
  other()
  await settle()
  expect(socket.closed).toBe(true)
})

test('a path outside the working tree is named, and covered once a live answers its message', async () => {
  const { files, reads, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  watch.show('/w/a.ts')
  const outside = watch.show('/etc/hosts')
  const socket = sockets[0]!
  socket.receive(live)
  await settle()
  expect(socket.sent).toEqual([{ type: 'paths', paths: ['/etc/hosts'] }])

  // Before the answer, a read there is not confirmed.
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  expect(reads).toEqual(['text /etc/hosts', 'text /etc/hosts'])

  socket.receive(live)
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  expect(reads).toHaveLength(3)

  // Named no more, it is no longer covered.
  outside()
  await settle()
  expect(socket.sent.at(-1)).toEqual({ type: 'paths', paths: [] })
  files.show(spec('text', '/etc/hosts')).release()
  await settle()
  expect(reads).toHaveLength(4)
})

test('lost reports, a Host that cannot watch and a closed socket leave nothing confirmed', async () => {
  const { files, reads, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  watch.show('/w/a.ts')
  const first = sockets[0]!
  first.receive(live)
  const shown = files.show(spec('text', '/w/a.ts'))
  await settle()

  // Lost: what shows is read again at once.
  first.receive({ type: 'state', state: 'lost' })
  await settle()
  expect(reads).toEqual(['text /w/a.ts', 'text /w/a.ts'])
  first.receive(live)

  first.receive({ type: 'state', state: 'unavailable', reason: 'inotify watch limit reached' })
  expect(watch.note.unavailable).toBe('inotify watch limit reached')
  shown.release()
  files.show(spec('text', '/w/a.ts')).release()
  await settle()
  expect(reads).toHaveLength(3)

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
