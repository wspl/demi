import { afterEach, beforeEach, expect, test } from 'bun:test'
import type { FileReads } from '@demicodes/web-ui/files/kept-source'
import { FileBrowserError } from '@demicodes/web-ui/files/types'
import type { UserStreamHandlers } from '@demicodes/web-ui/plugins/streams'
import { waitToReconnect } from '@demicodes/web-ui/transport/liveness'
import type { DirectAttempt } from '@demicodes/web-ui/devices/direct'
import type { ChannelHeader } from '../api/generated/web-api'
import { DeviceDirect } from './device'
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
  return { device, scope: { conversation: 'c1', cwd: '/work' } }
}

/** The relay's reads, each call recorded. */
function relayReads(calls: string[]): FileReads {
  return {
    platform: 'macos',
    home: '/home/ana',
    list: async (path) => {
      calls.push(`list ${path}`)
      return []
    },
    readText: async (path) => {
      calls.push(`text ${path}`)
      return { text: 'from the relay', version: 'W/"r"' }
    },
    upload: async (path) => {
      calls.push(`upload ${path}`)
    },
  }
}

test('a read runs on the direct channel while it is the choice, and again on the relay when the channel fails', async () => {
  const peer = new ScriptedPeer([
    {
      answer: { ok: true, version: 'W/"1"', unchanged: false },
      messages: [{ kind: 'bytes', bytes: new TextEncoder().encode('héllo') }],
    },
    { fails: new ChannelFailed('the peer went') },
  ])
  const route = await directOver(peer)
  const calls: string[] = []
  const reads = directFileReads(() => route, relayReads(calls))

  expect(await reads.readText!('/work/notes.md', null)).toEqual({ text: 'héllo', version: 'W/"1"' })
  expect(peer.headers[0]).toEqual({ conversation: 'c1', cwd: '/work', op: 'text', path: '/work/notes.md' })
  expect(calls).toEqual([])

  expect(await reads.readText!('/work/notes.md', 'W/"1"')).toEqual({ text: 'from the relay', version: 'W/"r"' })
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
