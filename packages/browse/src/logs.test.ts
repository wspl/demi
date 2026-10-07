// The page's logs as one daemon leaves them and the next goes on from them,
// in a temporary folder; a few milliseconds.
import { afterEach, expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Logs, recordDirectChannels } from './logs'

const folders: string[] = []

afterEach(() => {
  for (const folder of folders.splice(0)) {
    rmSync(folder, { recursive: true, force: true })
  }
})

/** The logs a daemon kept of the browser at `browser`, left in a file, and that file. */
function leftBehind(browser: string): string {
  const folder = mkdtempSync(join(tmpdir(), 'browse-logs-'))
  folders.push(folder)
  const path = join(folder, 'page-logs.json')
  const logs = new Logs()
  logs.add('console', 'log: before the mark')
  logs.mark('reload')
  logs.add('console', 'error: after the mark')
  logs.add('network', 'GET 200 http://127.0.0.1:3323/api/sync 12 ms')
  logs.save(path, browser)
  return path
}

test('a daemon that starts again with the same browser goes on from the logs and the mark the last one left', () => {
  const path = leftBehind('http://127.0.0.1:9222')
  const next = new Logs()
  next.restore(path, 'http://127.0.0.1:9222')
  next.add('console', 'log: after the restart')
  const lines = next.read('console', { all: false, modules: false }).map((line) => line.trim().replace(/^\S+ s\s+/, ''))
  expect([next.since(false), lines]).toEqual(['since the mark reload', ['error: after the mark', 'log: after the restart']])
  expect(next.read('console', { all: true, modules: false })).toHaveLength(3)
})

test('the logs of another browser are not the new browser\'s', () => {
  const path = leftBehind('http://127.0.0.1:9222')
  const next = new Logs()
  next.restore(path, 'http://127.0.0.1:9333')
  expect([next.read('console', { all: true, modules: false }), next.since(false)]).toEqual([[], 'since the tool attached to the browser'])
})

/** A data channel as the page's peer connection makes one: it sends, and hears the runner's messages. */
class FakeChannel extends EventTarget {
  readonly sent: unknown[] = []

  constructor(readonly label: string) {
    super()
  }

  send(data: unknown): void {
    this.sent.push(data)
  }

  hear(data: string | ArrayBuffer): void {
    this.dispatchEvent(new MessageEvent('message', { data }))
  }
}

class FakePeer {
  createDataChannel(label: string): FakeChannel {
    return new FakeChannel(label)
  }
}

test('operations a page runs on direct channels go with its requests, and a watch\'s messages with its sockets, a file\'s text with neither', () => {
  // Operations over a direct channel to a runner make no network event: without
  // the page's script, `log network` showed nothing once the page went direct.
  const lines: string[] = []
  const realPeer = Reflect.get(globalThis, 'RTCPeerConnection')
  const realDebug = console.debug
  Reflect.set(globalThis, 'RTCPeerConnection', FakePeer)
  console.debug = (line: string) => lines.push(line)
  try {
    recordDirectChannels('[browse direct]')
    // A second install, as by a daemon that attached again, records each once.
    recordDirectChannels('[browse direct]')
    const peer = new FakePeer()
    const text = peer.createDataChannel('text')
    text.send(JSON.stringify({ conversation: 'c1', op: 'text', path: '/w/notes.md' }))
    text.hear(JSON.stringify({ ok: true, version: 'v1', unchanged: false }))
    text.hear('the text')
    const watch = peer.createDataChannel('watch')
    watch.send(JSON.stringify({ conversation: 'c1', op: 'watch' }))
    watch.hear(JSON.stringify({ ok: true }))
    watch.send(JSON.stringify({ type: 'paths', paths: [] }))
    watch.hear(JSON.stringify({ type: 'changed', paths: ['/w/server.log'], ignored: ['/w/server.log'] }))
    const refused = peer.createDataChannel('list')
    refused.send(JSON.stringify({ conversation: 'c1', op: 'list', path: '/root' }))
    refused.hear(JSON.stringify({ error: { code: 'forbidden', message: 'No.' } }))
    expect(text.sent).toHaveLength(1)
  } finally {
    Reflect.set(globalThis, 'RTCPeerConnection', realPeer)
    console.debug = realDebug
  }

  const logs = new Logs()
  for (const line of lines) {
    logs.console(`debug: ${line}`)
  }
  logs.console('log: the page\'s own line')
  const read = (kind: 'console' | 'network' | 'sockets') =>
    logs.read(kind, { all: true, modules: false }).map((line) => line.trim().replace(/^\S+ s\s+/, '').replace(/ \d+ ms$/, ' _ ms'))
  expect(read('network')).toEqual([
    'direct text ok /w/notes.md _ ms',
    'direct watch ok _ ms',
    'direct list forbidden /root _ ms',
  ])
  expect(read('sockets')).toEqual([
    'sent direct watch {"type":"paths","paths":[]}',
    'received direct watch {"type":"changed","paths":["/w/server.log"],"ignored":["/w/server.log"]}',
  ])
  expect(read('console')).toEqual(['log: the page\'s own line'])
})
