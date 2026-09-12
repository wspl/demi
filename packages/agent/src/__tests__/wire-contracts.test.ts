import { expect, test } from 'bun:test'
import { PassThrough } from 'node:stream'
import { deferred, stringifyPortableJson, waitFor } from '@demicodes/utils'
import { AgentClient } from '../client/client'
import {
  createWebSocketClientTransport,
  createWebSocketServerTransport,
} from '../protocol/websocket-transport'
import { createStdioClientTransport, createStdioServerTransport } from '../protocol/stdio-transport'
import { serverFrameSchema } from '../protocol/server-schemas'
import { createMemoryWebSocketPair } from '../testing'
import { model } from './helpers'

const invalidFrames: unknown[] = [
  null,
  [],
  { type: 'send', messageId: 'wrong-direction', content: [] },
  { type: 'phase', phase: 'not-a-phase' },
  { type: 'pending_steers', pendingSteers: [{ id: 7 }] },
  { type: 'steer_result', steerId: 's', status: 'rejected' },
  { type: 'abort_result', result: { aborted: 'yes', target: null, canAbortAgain: false } },
  { type: 'transcript_reset', epoch: 'e', revision: -1, blocks: [] },
  { type: 'transcript_reset', epoch: 'e', revision: 0, blocks: [{ type: 'user' }] },
  {
    type: 'tool_progress',
    toolUseId: 't',
    output: [{ type: 'image', source: { data: 'AR==', mediaType: 'image/png' } }],
  },
  {
    type: 'subagent_transcript_patch',
    subagentId: 's',
    revision: 1,
    patches: [{ op: 'remove', path: ['blocks', -1] }],
  },
]

test('every consumed server frame is validated and wrong directions are rejected', () => {
  for (const frame of invalidFrames) expect(serverFrameSchema.safeParse(frame).success).toBe(false)
  expect(serverFrameSchema.parse({ type: 'phase', phase: 'idle', extra: 1 })).toEqual({
    type: 'phase',
    phase: 'idle',
  })
})

test('invalid WebSocket input rejects every kind of outstanding client wait and removes listeners', async () => {
  for (const payload of [
    '{"synthetic-secret":',
    ...invalidFrames.map((frame) => JSON.stringify(frame)),
  ]) {
    const [socket, peer] = createMemoryWebSocketPair()
    const client = new AgentClient(createWebSocketClientTransport(socket))
    const waits = [
      client.open({ providerId: model.providerId, model }, '/workspace', 'session'),
      client.send([{ type: 'text', text: 'send' }]),
      client.steer([{ type: 'text', text: 'steer' }]),
      client.abort(),
      client.shellWrite('command', 'input'),
      client.submit([{ type: 'text', text: 'submit' }]),
      client.editAndSend({
        operationId: 'edit',
        targetBlockId: 'user',
        version: { epoch: 'e', revision: 0 },
        content: [{ type: 'text', text: 'replacement' }],
      }),
    ]
    const settled = Promise.allSettled(waits)
    peer.send(payload)
    const results = await settled
    expect(results.every((result) => result.status === 'rejected')).toBe(true)
    for (const result of results) {
      if (result.status === 'rejected')
        expect(String(result.reason)).not.toContain('synthetic-secret')
    }
    expect(socket.listenerCount()).toBe(0)
    expect(client.transcriptVersion()).toBeNull()
  }
})

test('a nontext message or closed socket also settles client waits', async () => {
  for (const event of ['message', 'close', 'error'] as const) {
    const [socket] = createMemoryWebSocketPair()
    const client = new AgentClient(createWebSocketClientTransport(socket))
    const settled = Promise.allSettled([client.abort()])
    socket.deliver(event, new Uint8Array([1]))
    expect((await settled)[0]?.status).toBe('rejected')
    expect(socket.listenerCount()).toBe(0)
  }
})

test('valid frames wait for earlier async handlers and handler failures have an error outlet', async () => {
  const [socket, peer] = createMemoryWebSocketPair()
  const transport = createWebSocketServerTransport(socket)
  const release = deferred<void>()
  const entered = deferred<void>()
  const seen: string[] = []
  const errors: string[] = []
  transport.onError((error) => errors.push(error.code))
  transport.onFrame(async (frame) => {
    seen.push(frame.type)
    if (frame.type === 'abort') {
      entered.resolve()
      await release.promise
      throw new Error('synthetic handler failure')
    }
  })
  peer.send('{"type":"abort"}')
  peer.send('{"type":"close"}')
  await entered.promise
  expect(seen).toEqual(['abort'])
  release.resolve()
  await waitFor(() => seen.length === 2)
  expect(errors).toEqual(['handler_failed'])
  transport.close()
})

test('stdio distinguishes invalid frames from EOF, accepts CRLF and cleans up a failed client', async () => {
  const input = new PassThrough()
  const output = new PassThrough()
  const server = createStdioServerTransport(input, output)
  const errors: string[] = []
  const frames: string[] = []
  server.onError((error) => errors.push(error.code))
  server.onFrame((frame) => {
    frames.push(frame.type)
  })
  input.end('not-json\n{"type":"phase","phase":"idle"}\r\n{"type":"abort"}\r\n')
  await waitFor(() => errors.includes('closed'))
  expect(frames).toEqual(['abort'])
  expect(errors).toEqual(['invalid_frame', 'invalid_frame', 'closed'])
  expect(input.destroyed).toBe(true)
  expect(output.destroyed).toBe(true)
  expect(output.listenerCount('error')).toBe(0)

  const inbound = new PassThrough()
  const outbound = new PassThrough()
  const client = new AgentClient(createStdioClientTransport(inbound, outbound))
  const pending = Promise.allSettled([client.abort()])
  inbound.end(Buffer.from([34, 255, 34, 10]))
  expect((await pending)[0]?.status).toBe('rejected')
  expect(outbound.destroyed).toBe(true)
})

test('patch indices cannot create holes or append text to the wrong block kind', async () => {
  const block = {
    type: 'text' as const,
    id: 'text',
    createdAt: '2026-09-01T00:00:00.000Z',
    model,
    text: 'kept',
  }
  const [socket, peer] = createMemoryWebSocketPair()
  const client = new AgentClient(createWebSocketClientTransport(socket))
  peer.send(
    stringifyPortableJson({ type: 'transcript_reset', epoch: 'e', revision: 0, blocks: [block] }),
  )
  await waitFor(() => client.transcript().blocks.length === 1)
  peer.send(
    stringifyPortableJson({
      type: 'transcript_patch',
      revision: 1,
      patches: [{ op: 'remove', path: ['blocks', 4] }],
    }),
  )
  await waitFor(() => socket.listenerCount() === 0)
  expect(client.transcript().blocks).toEqual([block])
})
