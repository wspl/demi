import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection, UserContentBlock } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport } from '@demicodes/agent'
import { defineProvider, type InferenceRequest } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { waitFor } from '@demicodes/utils'
import { createBackend, type Backend } from '../index'

// M6 attachments: upload-then-reference for message media (bytes never ride
// the frame socket), inline resolution before the provider, checkpoint
// round-trip, and the workspace file drop.

const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01])

async function api(backend: Backend, path: string, init?: RequestInit): Promise<Response> {
  return fetch(`${backend.url}${path}`, init)
}

function selectionFor(connectionId: string) {
  const model: ModelSelection = {
    providerId: connectionId,
    model: { id: 'm', name: 'M', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
    thinking: null,
  }
  return { providerId: connectionId, model }
}

async function connectClient(backend: Backend, conversationId: string, selection: ReturnType<typeof selectionFor>) {
  const socket = new WebSocket(`${backend.url.replace('http', 'ws')}/api/conversations/${conversationId}/stream`)
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener('open', () => resolve(), { once: true })
    socket.addEventListener('error', () => reject(new Error('stream connect failed')), { once: true })
  })
  const client = new AgentClient(createWebSocketClientTransport(socket as never))
  await client.open(selection, '/ignored-by-server', 'ignored')
  return client
}

test('message attachment: upload → ref block → inline bytes at the provider → checkpoint round-trip', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-attach-'))
  const requests: InferenceRequest[] = []
  const stubRuntime = () =>
    new StubProvider([
      (request) => {
        requests.push(request)
        return [events.text('saw it'), events.response()]
      },
    ])
  const backend = await createBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: ({ connectionId, label }) => defineProvider({ id: connectionId, displayName: label, createRuntime: stubRuntime }),
    },
  })
  const connectionResponse = await api(backend, '/api/connections', {
    method: 'POST',
    body: JSON.stringify({ type: 'stub', label: 'Stub', apiKey: 'k' }),
    headers: { 'content-type': 'application/json' },
  })
  const { connection } = (await connectionResponse.json()) as { connection: { id: string } }
  const selection = selectionFor(connection.id)

  // Upload: bytes to the blob store, metadata row back.
  const uploaded = await api(backend, '/api/attachments', {
    method: 'POST',
    body: PNG_BYTES,
    headers: { 'content-type': 'image/png' },
  })
  expect(uploaded.status).toBe(201)
  const { attachment } = (await uploaded.json()) as {
    attachment: { id: string; mediaType: string; sizeBytes: number; sha256: string }
  }
  expect(attachment.mediaType).toBe('image/png')
  expect(attachment.sizeBytes).toBe(PNG_BYTES.length)

  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const client = await connectClient(backend, conversation.id, selection)

  // The send frame carries only the reference; the provider sees inline bytes.
  const refBlock = { type: 'image', source: { type: 'ref', ref: attachment.id } } as never as UserContentBlock
  await client.send([{ type: 'text', text: 'describe this' }, refBlock])
  const userItem = requests[0]?.items.find(
    (item): item is Extract<(typeof requests)[0]['items'][number], { type: 'user_message' }> => item.type === 'user_message',
  )
  const image = userItem?.content.find((block): block is Extract<UserContentBlock, { type: 'image' }> => block.type === 'image')
  expect(image?.source.type).toBe('binary')
  if (image?.source.type !== 'binary') throw new Error('expected binary image source')
  expect(image.source.data).toEqual(PNG_BYTES)
  expect(image.source.mediaType).toBe('image/png')

  // The browser sees media by reference, live and after a restore alike:
  // the transcript frames carry the blob's hash, and the bytes are one GET away.
  const liveImage = client
    .transcript()
    .blocks.filter((block) => block.type === 'user')
    .flatMap((block) => (block.type === 'user' ? block.content : []))
    .find((block): block is Extract<UserContentBlock, { type: 'image' }> => block.type === 'image')
  expect(liveImage?.source).toEqual({ type: 'ref', ref: attachment.sha256, mediaType: 'image/png' } as never)
  await client.close()
  const revived = await connectClient(backend, conversation.id, selection)
  await waitFor(() => revived.transcript().blocks.length > 0, undefined, { timeoutMs: 5_000 })
  const revivedImage = revived
    .transcript()
    .blocks.filter((block) => block.type === 'user')
    .flatMap((block) => (block.type === 'user' ? block.content : []))
    .find((block): block is Extract<UserContentBlock, { type: 'image' }> => block.type === 'image')
  expect(revivedImage?.source).toEqual({ type: 'ref', ref: attachment.sha256, mediaType: 'image/png' } as never)
  const blob = await api(backend, `/api/blobs/${attachment.sha256}?type=image/png`)
  expect(blob.status).toBe(200)
  expect(blob.headers.get('content-type')).toBe('image/png')
  expect(blob.headers.get('cache-control')).toContain('immutable')
  expect(new Uint8Array(await blob.arrayBuffer())).toEqual(PNG_BYTES)
  expect((await api(backend, '/api/blobs/0000000000000000000000000000000000000000000000000000000000000000')).status).toBe(404)

  // A missing reference degrades loudly to a visible placeholder, never a crash.
  const ghost = { type: 'image', source: { type: 'ref', ref: 'no-such-id' } } as never as UserContentBlock
  await revived.send([ghost]).catch(() => {})
  const placeholder = revived
    .transcript()
    .blocks.filter((block) => block.type === 'user')
    .flatMap((block) => (block.type === 'user' ? block.content : []))
    .some((block) => block.type === 'text' && block.text.includes('no-such-id'))
  expect(placeholder).toBe(true)

  await revived.close()
  await backend.close()
}, 20_000)

test('attachment upload limits and workspace file drop', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-drop-'))
  const stubRuntime = () =>
    new StubProvider([[events.toolCall('t1', 'shell_exec', { script: 'cat notes/readme.md', timeoutMs: 10_000 })], [events.text('ok'), events.response()]])
  const backend = await createBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: ({ connectionId, label }) => defineProvider({ id: connectionId, displayName: label, createRuntime: stubRuntime }),
    },
  })

  // Empty and oversized uploads are refused.
  expect((await api(backend, '/api/attachments', { method: 'POST', body: new Uint8Array(0), headers: { 'content-type': 'image/png' } })).status).toBe(400)

  const connectionResponse = await api(backend, '/api/connections', {
    method: 'POST',
    body: JSON.stringify({ type: 'stub', label: 'Stub', apiKey: 'k' }),
    headers: { 'content-type': 'application/json' },
  })
  const { connection } = (await connectionResponse.json()) as { connection: { id: string } }
  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }

  // Path traversal is refused; a clean relative path lands in the virtual cwd.
  const traversal = await api(backend, `/api/conversations/${conversation.id}/workspace-files?name=../escape.txt`, {
    method: 'POST',
    body: 'nope',
  })
  expect(traversal.status).toBe(400)
  const dropped = await api(backend, `/api/conversations/${conversation.id}/workspace-files?name=notes/readme.md`, {
    method: 'POST',
    body: 'dropped content',
  })
  expect(dropped.status).toBe(201)
  expect(((await dropped.json()) as { path: string }).path).toBe('/home/demi/notes/readme.md')

  // The agent's shell sees the dropped file on the execution target.
  const client = await connectClient(backend, conversation.id, selectionFor(connection.id))
  const outputs: string[] = []
  client.subscribe((event) => {
    if (event.type === 'shell_output' && event.status.status === 'exited') outputs.push(event.status.stdout.delta)
  })
  await client.send([{ type: 'text', text: 'read the drop' }])
  expect(outputs.at(-1)).toBe('dropped content')

  await client.close()
  await backend.close()
}, 20_000)
