import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { HostFiles, SUMMARY_REREAD_MS, type KeptSpec } from '@demicodes/web-ui/files/file-cache'
import type { FileWatchMessage, FileWatchRequest } from '../api/generated/web-api'
import { ConversationWatch, deviceWatch } from './file-watch'

// The page's file watches as the backend plays them (`web-api.md` § File
// watch): the socket each opens, the subscriptions it sends for what the
// page keeps, and how it connects again, moves and lets go.
// Cost: no I/O, a fake WebSocket and fake timers; a few milliseconds each.

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
  for (let turn = 0; turn < 20; turn++)
    await Promise.resolve()
}

/** The Host's kept files, each read counted and answered at once, at the Host's number `at`. */
function kept() {
  const files = new HostFiles()
  const reads: string[] = []
  const host = { at: 1 }
  const spec = (kind: KeptSpec<string>['kind'], path: string, watch: KeptSpec<string>['watch']): KeptSpec<string> => ({
    kind,
    path,
    watch,
    read: async () => {
      reads.push(`${kind} ${path}`)
      return { value: `${reads.length}`, readAt: { epoch: 'e', number: host.at } }
    },
    size: (text) => text.length,
  })
  return { files, reads, spec, host }
}

/** Each subscription the socket sent, by its kind and path, with its key. */
function subscribed(socket: WatchSocket): { key: string; kind: string; path: string }[] {
  return socket.sent.flatMap((request) => request.type === 'subscribe' ? request.subscriptions : [])
}

test("a conversation's watch subscribes to what is kept, and a change reads what shows again", async () => {
  jest.useFakeTimers()
  try {
    const { files, reads, spec } = kept()
    const watch = new ConversationWatch('c1')
    const channel = watch.channel('d1', files)
    files.show(spec('changes', '/w', channel))
    files.show(spec('sides', '/w/a.ts', channel)).release()
    await flush()
    const socket = sockets[0]!
    expect(new URL(socket.url).pathname).toBe('/api/conversations/c1/fs/watch')
    socket.open()
    await flush()
    expect(subscribed(socket).map(({ kind, path }) => `${kind} ${path}`)).toEqual(['worktree /w', 'file /w/a.ts'])
    for (const { key } of subscribed(socket))
      socket.receive({ type: 'live', key, since: 1, epoch: 'e' })
    await flush()
    expect(reads).toEqual(['changes /w', 'sides /w/a.ts'])

    // The working tree changed: the list on screen is read again once the
    // second since its last read ends; the sides no view shows are not.
    const tree = subscribed(socket)[0]!.key
    socket.receive({ type: 'changed', changes: [[tree, 2], [subscribed(socket)[1]!.key, 2]] })
    jest.advanceTimersByTime(SUMMARY_REREAD_MS)
    await flush()
    expect(reads).toEqual(['changes /w', 'sides /w/a.ts', 'changes /w'])
  } finally {
    jest.useRealTimers()
  }
})

test('a watch that closes connects again after its wait, at once when the Host is back, and subscribes again', async () => {
  const { files, spec } = kept()
  const watch = new ConversationWatch('c1')
  files.show(spec('text', '/w/a.ts', watch.channel('d1', files)))
  await flush()
  const first = sockets[0]!
  first.open()
  await flush()

  first.receive({ type: 'offline' })
  first.end()
  watch.online()
  expect(sockets).toHaveLength(2)
  sockets[1]!.open()
  await flush()
  expect(subscribed(sockets[1]!)).toEqual(subscribed(first))
})

test('a watch that brings nothing, not even a heartbeat, for 75 s is broken and connects again', async () => {
  jest.useFakeTimers()
  try {
    const { files, spec } = kept()
    const watch = new ConversationWatch('c1')
    files.show(spec('text', '/w/a.ts', watch.channel('d1', files)))
    await flush()
    const first = sockets[0]!
    first.open()
    await flush()
    // Heartbeats keep a quiet watch.
    jest.advanceTimersByTime(60_000)
    first.receive({ type: 'heartbeat' })
    jest.advanceTimersByTime(74_999)
    expect(first.closed).toBe(false)
    // Silent for 75 s: broken, and closed.
    jest.advanceTimersByTime(1)
    // The judgement waits for the messages already queued.
    jest.advanceTimersByTime(0)
    expect(first.closed).toBe(true)
    // The first wait is at most a second.
    jest.advanceTimersByTime(1_000)
    expect(sockets).toHaveLength(2)
  } finally {
    jest.useRealTimers()
  }
})

test('a watch moves to the other path at once, and what it kept is current again once live and the numbers say', async () => {
  const { files, reads, spec, host } = kept()
  const watch = new ConversationWatch('c1')
  const channel = watch.channel('d1', files)
  files.show(spec('text', '/w/a.ts', channel)).release()
  await flush()
  const first = sockets[0]!
  first.open()
  await flush()
  const key = subscribed(first)[0]!.key
  first.receive({ type: 'live', key, since: 1, epoch: 'e' })
  files.show(spec('text', '/w/a.ts', channel)).release()
  await flush()
  expect(reads).toHaveLength(1)

  // The path to the Host changed: the next connection opens without a wait
  // and subscribes again; until it is live, what was read is checked.
  watch.move()
  expect(first.closed).toBe(true)
  expect(sockets).toHaveLength(2)
  const second = sockets[1]!
  second.open()
  await flush()
  expect(subscribed(second)).toEqual([{ key, kind: 'file', path: '/w/a.ts' }])
  files.show(spec('text', '/w/a.ts', channel)).release()
  await flush()
  expect(reads).toHaveLength(2)
  host.at = 3
  second.receive({ type: 'live', key, since: 2, epoch: 'e' })
  files.show(spec('text', '/w/a.ts', channel)).release()
  await flush()
  files.show(spec('text', '/w/a.ts', channel)).release()
  await flush()
  expect(reads).toHaveLength(3)
})

test('a conversation that moves to another Host lets go of what it kept there, and nothing kept closes the watch', async () => {
  const { files, spec } = kept()
  const watch = new ConversationWatch('c1')
  const shown = files.show(spec('text', '/w/a.ts', watch.channel('d1', files)))
  await flush()
  sockets[0]!.open()
  await flush()

  const other = new HostFiles()
  watch.channel('d2', other)
  expect(sockets[0]!.closed).toBe(true)
  expect(shown.entry.unwatched).toBeNull()
  await settle()
  expect(sockets).toHaveLength(1)
})

test("a device's folder dialog subscribes to the details of what it lists on the device's own watch", async () => {
  const { files } = kept()
  const listing = files.show({
    kind: 'listing',
    path: '/Users/alex/logs',
    details: true,
    watch: deviceWatch('d7', files),
    read: async () => ({ value: [], readAt: { epoch: 'e', number: 1 } }),
    size: () => 0,
  })
  await flush()
  const socket = sockets[0]!
  expect(new URL(socket.url).pathname).toBe('/api/devices/d7/fs/watch')
  socket.open()
  await flush()
  expect(subscribed(socket).map(({ kind, path }) => `${kind} ${path}`)).toEqual(['folder_details /Users/alex/logs'])
  listing.release()
})
