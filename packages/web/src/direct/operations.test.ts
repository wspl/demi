import { afterEach, beforeEach, expect, test } from 'bun:test'
import type { FileReads } from '@demicodes/web-ui/files/kept-source'
import { FileBrowserError } from '@demicodes/web-ui/files/types'
import type { UserStreamHandlers } from '@demicodes/web-ui/plugins/streams'
import { waitToReconnect } from '@demicodes/web-ui/transport/liveness'
import type { DirectAttempt } from '@demicodes/web-ui/devices/direct'
import type { ChannelHeader } from '../api/generated/web-api'
import { DeviceDirect } from './device'
import { DeviceLive, LIVE_IDLE_MS } from './live'
import { encodeAcks, encodeControl, framed } from '@demicodes/web-ui/live/frames'
import { LIVE_ACK_FRAME, LIVE_CONTROL_FRAME, LIVE_VIDEO_FRAME } from '@demicodes/protocol'
import type { DirectRoute } from '.'
import { directFileReads, directStream } from './operations'
import { ChannelFailed, ChannelRefused, type ChannelMessage, type DirectPeer, type OperationChannel } from './peer'

// A conversation's operations over either path (`direct-channel.md`
// § Choosing the path), on a peer and a relay the test plays: each starts
// on the choice of its start; one that fails on the channel sets the relay
// and runs again there, a write once; a refusal is the relay's answer; a
// stream moves when the choice changes, without a reconnect wait.

const realWindow = Reflect.get(globalThis, 'window')
beforeEach(() => {
  if (realWindow === undefined)
    Reflect.set(globalThis, 'window', { location: { href: 'http://127.0.0.1:3271/chat' } })
})
afterEach(() => {
  Reflect.set(globalThis, 'window', realWindow)
})

/** How the test answers one channel: its answer and messages, or a failure. */
type Script = { answer: unknown; messages?: ChannelMessage[] } | { fails: Error }

/** A connected peer that answers each channel as the test scripted it, in order, and records each header and what was sent. */
class ScriptedPeer implements DirectPeer {
  readonly headers: ChannelHeader[] = []
  readonly sent: (string | Uint8Array)[] = []
  readonly ended = Promise.withResolvers<void>()
  readonly closed = this.ended.promise
  readonly attempt: DirectAttempt = {
    startedAt: '2026-10-08T09:00:00.000Z',
    durationMs: 20,
    outcome: 'connected',
    stage: null,
    browser: { local: [], public: [] },
    device: { local: ['127.0.0.1'], public: [] },
    pairs: { tried: 1, answered: 1 },
    inUse: null,
    permission: null,
  }
  constructor(private readonly scripts: Script[]) {}

  probe(): void {}
  onProbe(): () => void {
    return () => {}
  }

  async open<T>(header: ChannelHeader): Promise<OperationChannel<T>> {
    this.headers.push(header)
    const script = this.scripts.shift()
    if (!script)
      throw new Error(`no script for ${header.op}`)
    const messages = 'fails' in script ? [] : [...(script.messages ?? [])]
    return {
      answer: async () => {
        if ('fails' in script)
          throw script.fails
        return script.answer as T
      },
      next: async () => messages.shift() ?? null,
      sendText: async (text) => {
        this.sent.push(text)
      },
      sendBytes: async (bytes) => {
        this.sent.push(bytes)
      },
      close: () => {},
    }
  }

  close(): void {
    this.ended.resolve()
  }
}

/** A device whose choice is direct over `peer`. */
async function directOver(peer: DirectPeer): Promise<DirectRoute> {
  const device = new DeviceDirect({ ready: () => true, connect: async () => peer, after: () => () => {}, connected: () => {} })
  device.tryNow()
  await Promise.resolve()
  await Promise.resolve()
  expect(device.choice).toBe('direct')
  const live = new DeviceLive({ connect: async () => peer, closed: () => {}, after: () => () => {} })
  return { device, live, scope: { conversation: 'c1', cwd: '/work' } }
}

/** The relay's reads, each call recorded. */
function relayReads(calls: string[]): FileReads {
  return {
    platform: 'macos',
    home: '/home/ana',
    list: async (path) => {
      calls.push(`list ${path}`)
      return { value: [], readAt: null }
    },
    readText: async (path) => {
      calls.push(`text ${path}`)
      return { value: { text: 'from the relay', version: 'W/"r"' }, readAt: { epoch: 'e', number: 9 } }
    },
    upload: async (path) => {
      calls.push(`upload ${path}`)
    },
  }
}

test('a read runs on the direct channel while it is the choice, and again on the relay when the channel fails', async () => {
  const peer = new ScriptedPeer([
    {
      answer: { ok: true, version: 'W/"1"', unchanged: false, readAt: { epoch: 'e', number: 3 } },
      messages: [{ kind: 'bytes', bytes: new TextEncoder().encode('héllo') }],
    },
    { fails: new ChannelFailed('the peer went') },
  ])
  const route = await directOver(peer)
  const calls: string[] = []
  const reads = directFileReads(() => route, relayReads(calls))

  expect(await reads.readText!('/work/notes.md', null)).toEqual({
    value: { text: 'héllo', version: 'W/"1"' },
    readAt: { epoch: 'e', number: 3 },
  })
  expect(peer.headers[0]).toEqual({ conversation: 'c1', cwd: '/work', op: 'text', path: '/work/notes.md' })
  expect(calls).toEqual([])

  expect(await reads.readText!('/work/notes.md', 'W/"1"')).toEqual({
    value: { text: 'from the relay', version: 'W/"r"' },
    readAt: { epoch: 'e', number: 9 },
  })
  expect(calls).toEqual(['text /work/notes.md'])
  expect(route.device.choice).toBe('relay')
  // The next read starts on the relay.
  await reads.list('/work')
  expect(calls).toEqual(['text /work/notes.md', 'list /work'])
  expect(peer.headers).toHaveLength(2)
})

test('a refusal is the relay route\'s answer, and busy sends the operation to the relay without leaving the channel', async () => {
  const peer = new ScriptedPeer([
    { fails: new ChannelRefused('fs_error', 404, 'No such file') },
    { fails: new ChannelRefused('busy', 503, 'Too many channels') },
  ])
  const route = await directOver(peer)
  const calls: string[] = []
  const reads = directFileReads(() => route, relayReads(calls))

  const missing = await reads.readText!('/work/gone.md', null).catch((error: unknown) => error)
  expect(missing).toBeInstanceOf(FileBrowserError)
  expect(missing).toMatchObject({ kind: 'not-found' })
  expect(calls).toEqual([])

  await reads.list('/work')
  expect(calls).toEqual(['list /work'])
  expect(route.device.choice).toBe('direct')
})

test('a write sends its bytes and its end, and one that fails is retried once on the relay', async () => {
  const peer = new ScriptedPeer([
    { answer: { ok: true } },
    { fails: new ChannelFailed('the peer went') },
  ])
  const route = await directOver(peer)
  const calls: string[] = []
  const reads = directFileReads(() => route, relayReads(calls))
  const sent: number[] = []
  const options = { replace: false, signal: new AbortController().signal, progress: (bytes: number) => sent.push(bytes) }

  await reads.upload!('/work/a.txt', new File(['abc'], 'a.txt'), options)
  expect(peer.headers[0]).toEqual({ conversation: 'c1', cwd: '/work', op: 'write', path: '/work/a.txt', replace: false })
  expect(peer.sent.map((part) => (typeof part === 'string' ? part : new TextDecoder().decode(part)))).toEqual(['abc', '{"end":true}'])
  expect(sent).toEqual([3])
  expect(calls).toEqual([])

  await reads.upload!('/work/b.txt', new File(['def'], 'b.txt'), options)
  expect(calls).toEqual(['upload /work/b.txt'])
})

test('a stream moves to the other path when the choice changes, and its view connects again at once', async () => {
  const peer = new ScriptedPeer([{ answer: { ok: true }, messages: [] }])
  const route = await directOver(peer)
  const relayOpened: string[] = []
  const relay = (handlers: UserStreamHandlers) => {
    relayOpened.push('relay')
    void handlers
    return { send: () => {}, close: () => {} }
  }
  const open = directStream(() => route, 'browser', relay)
  const closed: string[] = []
  // A view that ends waits to reconnect, as the live view does.
  let reconnected = false
  open({
    data: () => {},
    closed: (reason) => {
      closed.push(reason)
      waitToReconnect(1, () => {
        reconnected = true
      })
    },
  })
  await Promise.resolve()
  expect(peer.headers[0]).toEqual({ conversation: 'c1', cwd: '/work', op: 'stream', stream: 'browser' })
  expect(relayOpened).toEqual([])

  route.device.failed()
  expect(closed).toEqual(['moved'])
  expect(reconnected).toBe(true)
  // Reopened while the choice is the relay, the view opens there.
  open({ data: () => {}, closed: () => {} })
  expect(relayOpened).toEqual(['relay'])
})

test('a stream the runner refuses opens on the relay, and the channel stays the choice', async () => {
  const peer = new ScriptedPeer([{ fails: new ChannelRefused('busy', 503, 'Too many channels') }])
  const route = await directOver(peer)
  const relayOpened: string[] = []
  const relay = (handlers: UserStreamHandlers) => {
    relayOpened.push('relay')
    void handlers
    return { send: () => {}, close: () => {} }
  }
  const closed: string[] = []
  directStream(() => route, 'browser', relay)({ data: () => {}, closed: (reason) => closed.push(reason) })
  for (let turn = 0; turn < 5; turn++)
    await Promise.resolve()
  expect(peer.headers[0]).toMatchObject({ op: 'stream', stream: 'browser' })
  expect(relayOpened).toEqual(['relay'])
  expect(closed).toEqual([])
  expect(route.device.choice).toBe('direct')
})

/** A channel the test speaks for: its answer, the runner's messages as the test gives them, and what the page sent. */
class PlayedChannel<T> implements OperationChannel<T> {
  readonly sent: Uint8Array[] = []
  closed = false
  private readonly messages: (ChannelMessage | null)[] = []
  private waiting: ((message: ChannelMessage | null) => void) | null = null
  constructor(private readonly answered: T) {}

  /** The runner sends `bytes`, or closes the channel. */
  give(bytes: Uint8Array | null): void {
    const message: ChannelMessage | null = bytes && { kind: 'bytes', bytes }
    if (this.waiting) {
      this.waiting(message)
      this.waiting = null
    } else {
      this.messages.push(message)
    }
  }

  answer = async () => this.answered
  next(): Promise<ChannelMessage | null> {
    if (this.messages.length)
      return Promise.resolve(this.messages.shift()!)
    return new Promise((resolve) => {
      this.waiting = resolve
    })
  }
  sendText = async () => {}
  sendBytes = async (bytes: Uint8Array) => {
    this.sent.push(bytes)
  }
  close(): void {
    this.closed = true
  }
}

/** A live peer whose channels the test plays, each header and whether it is ordered recorded. */
class LivePeer extends ScriptedPeer {
  readonly opened: { header: ChannelHeader; ordered: boolean; channel: PlayedChannel<unknown> }[] = []
  override async open<T>(header: ChannelHeader, _answer?: unknown, ordered = true): Promise<OperationChannel<T>> {
    const channel = new PlayedChannel<unknown>(header.op === 'video' ? { ok: true, id: 3 } : { ok: true })
    this.opened.push({ header, ordered, channel })
    return channel as OperationChannel<T>
  }
}

async function settle(): Promise<void> {
  for (let turn = 0; turn < 10; turn++)
    await Promise.resolve()
}

/**
 * A live view's stream on the direct path (`live-view.md` § Two paths, one
 * protocol): its unordered `video` channel opens first, its `stream` names
 * it; the page's acknowledgements go on `video` and the rest on `stream`;
 * the view reads the chunks from `video` and whole frames from `stream`,
 * even where a frame came in two messages. Planted defects this catches:
 * an ordered `video`, acknowledgements on `stream`, a frame of `stream`
 * handed on in pieces between which a chunk lands.
 */
test('a live view takes the live peer, its pictures and acknowledgements on its video channel', async () => {
  const files = new ScriptedPeer([])
  const route = await directOver(files)
  const peer = new LivePeer([])
  const closedPeers: string[] = []
  let idle: (() => void) | null = null
  route.live = new DeviceLive({
    connect: async () => peer,
    closed: () => closedPeers.push('live'),
    after: (ms, run) => {
      expect(ms).toBe(LIVE_IDLE_MS)
      idle = run
      return () => {
        idle = null
      }
    },
  })
  const heard: number[][] = []
  const relay = () => ({ send: () => {}, close: () => {} })
  const stream = directStream(() => route, 'browser', relay)({ data: (bytes) => heard.push([...bytes]), closed: () => {} }, { live: true })
  stream.send(encodeControl({ type: 'hello' }))
  await settle()
  expect(peer.opened.map(({ header, ordered }) => [header, ordered])).toEqual([
    [{ op: 'video' }, false],
    [{ conversation: 'c1', cwd: '/work', op: 'stream', stream: 'browser', video: 3 }, true],
  ])
  expect(files.headers).toEqual([])
  const [video, control] = peer.opened.map(({ channel }) => channel) as [PlayedChannel<unknown>, PlayedChannel<unknown>]
  const ack = encodeAcks([{ generation: 1, sequence: 1, chunk: 0, received: 5 }])
  stream.send(ack)
  expect(video.sent.map((bytes) => bytes[4])).toEqual([LIVE_ACK_FRAME])
  expect(control.sent.map((bytes) => bytes[4])).toEqual([LIVE_CONTROL_FRAME])

  const state = encodeControl({ type: 'state' })
  const chunk = framed(LIVE_VIDEO_FRAME, new Uint8Array([9, 9]))
  control.give(state.subarray(0, 7))
  await settle()
  video.give(chunk)
  await settle()
  control.give(state.subarray(7))
  await settle()
  expect(heard).toEqual([[...chunk], [...state]])

  // The view's end closes both channels, and the peer a minute later.
  stream.close()
  expect([video.closed, control.closed]).toEqual([true, true])
  expect(closedPeers).toEqual([])
  idle!()
  expect(closedPeers).toEqual(['live'])
})

/**
 * A live view whose live peer cannot be made opens on the relay, which
 * gets what the view said first, as its hello, before the fall back.
 * Planted defect this catches: the view's first messages left in the
 * direct stream's queue, so the module never hears the view.
 */
test('a live view whose live peer cannot be made opens on the relay with what it said first', async () => {
  const route = await directOver(new ScriptedPeer([]))
  route.live = new DeviceLive({ connect: async () => Promise.reject(new ChannelFailed('no')), closed: () => {}, after: () => () => {} })
  const relayed: string[] = []
  const relay = () => {
    relayed.push('opened')
    return { send: (bytes: Uint8Array) => relayed.push(new TextDecoder().decode(bytes.subarray(5))), close: () => {} }
  }
  const stream = directStream(() => route, 'browser', relay)({ data: () => {}, closed: () => {} }, { live: true })
  stream.send(encodeControl({ type: 'hello' }))
  stream.send(encodeControl({ type: 'watch' }))
  await settle()
  stream.send(encodeControl({ type: 'input' }))
  expect(relayed).toEqual(['opened', '{"type":"hello"}', '{"type":"watch"}', '{"type":"input"}'])
  expect(route.device.choice).toBe('direct')
})
