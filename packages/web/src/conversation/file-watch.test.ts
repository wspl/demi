import { afterEach, beforeEach, expect, test } from 'bun:test'
import { HostFiles, type KeptSpec } from '@demicodes/web-ui/files/file-cache'
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

test('a live watch confirms what is read, and a report has what it names read again', async () => {
  const { files, reads, spec } = kept()
  const watch = new ConversationWatch('c1', () => ({ files, root: '/w' }))
  const unfollow = watch.show('/w/a.ts')
  const socket = sockets[0]!
  expect(new URL(socket.url).pathname).toBe('/api/conversations/c1/fs/watch')
  socket.receive(live)

  const shown = files.show(spec('text', '/w/a.ts'))
  await settle()
  shown.release()
  files.show(spec('text', '/w/a.ts')).release()
  expect(reads).toEqual(['text /w/a.ts'])

  const again = files.show(spec('text', '/w/a.ts'))
  socket.receive({ type: 'changed', paths: ['/w/a.ts'] })
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
