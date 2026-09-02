import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport } from '@demicodes/agent'
import { defineProvider, type ProviderEvent } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { delay, waitFor } from '@demicodes/utils'
import { createBackend, type Backend } from '../index'

// M2 acceptance: a zero-setup virtual conversation over the real Web API +
// conversation stream — portable-command tools work, real programs surface
// upgrade guidance, a detached client's turn completes server-side and a
// reattach sees the full result, and cold history equals the live transcript.

function selectionFor(connectionId: string) {
  const model: ModelSelection = {
    providerId: connectionId,
    model: {
      id: 'test-model',
      name: 'Test Model',
      contextWindow: 100_000,
      inputLimit: null,
      thinking: [],
      acceptedExtensions: [],
    },
    thinking: null,
  }
  return { providerId: connectionId, model }
}

/** Registers `type: 'stub'` connections backed by the given runtime factory. */
function stubTypes(createRuntime: () => import('@demicodes/provider').AgentProvider) {
  return {
    stub: ({ connectionId, label }: { connectionId: string; label: string }) =>
      defineProvider({ id: connectionId, displayName: label, createRuntime }),
  }
}

async function createStubConnection(backend: Backend): Promise<string> {
  const response = await fetch(`${backend.url}/api/connections`, {
    method: 'POST',
    body: JSON.stringify({ type: 'stub', label: 'Stub', apiKey: 'test-key' }),
    headers: { 'content-type': 'application/json' },
  })
  if (response.status !== 201) throw new Error(`connection create failed: ${response.status}`)
  const { connection } = (await response.json()) as { connection: { id: string } }
  return connection.id
}

function slowTextTurn(text: string): AsyncIterable<ProviderEvent> {
  return (async function* () {
    for (const word of text.split(' ')) {
      await delay(30)
      yield { type: 'text_delta', text: `${word} ` } satisfies ProviderEvent
    }
    yield events.response()
  })()
}

async function api<T>(backend: Backend, path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${backend.url}${path}`, init)
  if (!response.ok) throw new Error(`${path}: HTTP ${response.status} ${await response.text()}`)
  return (await response.json()) as T
}

async function connectClient(
  backend: Backend,
  conversationId: string,
): Promise<{ client: AgentClient; socket: WebSocket }> {
  const socket = new WebSocket(`${backend.url.replace('http', 'ws')}/api/conversations/${conversationId}/stream`)
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener('open', () => resolve(), { once: true })
    socket.addEventListener('error', () => reject(new Error('stream connect failed')), { once: true })
  })
  return { client: new AgentClient(createWebSocketClientTransport(socket as never)), socket }
}

test('hostless conversation end-to-end: tinybash builtins, refusal, detach-safe turns, cold history', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-backend-'))
  const stub = new StubProvider([
    // Turn 1: tinybash builtins over the hostless tree.
    [events.toolCall('t1', 'shell_exec', { script: 'echo -n hello > f.txt && cat f.txt', timeoutMs: 10_000 })],
    [events.text('files done'), events.response()],
    // Turn 2: a real program puts the script outside the subset; nothing runs.
    [events.toolCall('t2', 'shell_exec', { script: 'python3 -V', timeoutMs: 10_000 })],
    [events.text('refused as expected'), events.response()],
  ])
  const backend = await createBackend({ dataDir, port: 0, providerTypes: stubTypes(() => stub) })
  const connectionId = await createStubConnection(backend)

  // Web API basics.
  const me = await api<{ user: { id: string; role: string } }>(backend, '/api/auth/me')
  expect(me.user.role).toBe('master')
  const models = await api<{ connections: Array<{ connectionId: string }> }>(backend, '/api/models')
  expect(models.connections.map((connection) => connection.connectionId)).toEqual([connectionId])
  const selection = selectionFor(connectionId)

  const created = await api<{ conversation: { id: string; title: string } }>(backend, '/api/conversations', {
    method: 'POST',
  })
  const conversationId = created.conversation.id
  expect(created.conversation.title).toBe('New conversation')

  // Live stream: the browser names no cwd — the server scopes the session.
  const { client } = await connectClient(backend, conversationId)
  const shellOutputs: string[] = []
  const shellErrors: string[] = []
  client.subscribe((event) => {
    if (event.type === 'shell_output' && event.status.status === 'exited') {
      shellOutputs.push(event.status.stdout.delta)
      shellErrors.push(event.status.stderr.delta)
    }
  })
  await client.open(selection, '/ignored-by-server', 'ignored-session-id')

  await client.send([{ type: 'text', text: 'work with files please' }])
  expect(shellOutputs[0]).toBe('hello')

  await client.send([{ type: 'text', text: 'now run python' }])
  expect(shellErrors[1]).toBe('tinybash: line 1: python3: no such program here; a machine\n')

  // The first user message became the title.
  const listed = await api<{ conversations: Array<{ id: string; title: string }> }>(backend, '/api/conversations')
  expect(listed.conversations.find((row) => row.id === conversationId)?.title).toBe('work with files please')

  await client.close()
  await backend.close()
}, 30_000)

test('detach mid-turn: the turn completes server-side and a reattach sees the result; cold history matches', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-backend-detach-'))
  const stub = new StubProvider([
    () => [events.text('warm up'), events.response()],
  ])
  // A provider whose second turn streams slowly so the client can detach mid-turn.
  let turn = 0
  const backend = await createBackend({
    dataDir,
    port: 0,
    providerTypes: stubTypes(() => {
      const runtime: import('@demicodes/provider').AgentProvider = {
        run(request) {
          turn += 1
          if (turn === 1) return stub.run(request)
          return slowTextTurn('the complete slow answer arrived intact')
        },
        clone: () => runtime,
      }
      return runtime
    }),
  })
  const connectionId = await createStubConnection(backend)
  const selection = selectionFor(connectionId)
  const created = await api<{ conversation: { id: string } }>(backend, '/api/conversations', { method: 'POST' })
  const conversationId = created.conversation.id

  const { client, socket } = await connectClient(backend, conversationId)
  await client.open(selection, '/ignored', 'ignored')
  await client.send([{ type: 'text', text: 'hi' }])

  // Start the slow turn and drop the raw socket while it streams — a browser
  // refresh, not an explicit protocol close.
  void client.send([{ type: 'text', text: 'now be slow' }]).catch(() => {})
  await delay(60)
  socket.close()

  // Reattach: a fresh socket + open on the same conversation adopts the live
  // session; the server-side turn finishes untouched by the binding close.
  const { client: client2 } = await connectClient(backend, conversationId)
  await client2.open(selection, '/ignored', 'ignored')
  await waitFor(
    () =>
      client2
        .transcript()
        .blocks.some(
          (block) => block.type === 'text' && block.text.includes('the complete slow answer arrived intact'),
        ),
    () => `blocks: ${JSON.stringify(client2.transcript().blocks.map((block) => block.type))}`,
    { timeoutMs: 10_000 },
  )
  const liveBlocks = client2.transcript().blocks

  // Cold history rides the same data: the REST read equals the live transcript.
  const cold = await api<{ blocks: Block[] }>(backend, `/api/conversations/${conversationId}/transcript`)
  expect(cold.blocks.map((block) => block.id)).toEqual(liveBlocks.map((block) => block.id))
  expect(cold.blocks).toHaveLength(liveBlocks.length)

  await client2.close()
  await backend.close()
}, 30_000)

test('backend restart restores the conversation from its database (M3)', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-backend-restart-'))
  const types = () =>
    stubTypes(
      () =>
        new StubProvider([
          [events.text('first answer'), events.response()],
          [events.text('second answer'), events.response()],
        ]),
    )

  const backend = await createBackend({ dataDir, port: 0, providerTypes: types() })
  const connectionId = await createStubConnection(backend)
  const selection = selectionFor(connectionId)
  const created = await api<{ conversation: { id: string } }>(backend, '/api/conversations', { method: 'POST' })
  const conversationId = created.conversation.id
  const { client } = await connectClient(backend, conversationId)
  await client.open(selection, '/ignored', 'ignored')
  await client.send([{ type: 'text', text: 'remember me' }])
  const before = client.transcript().blocks
  expect(before.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  await client.close()
  await backend.close()

  // A fresh process over the same data directory: cold history and the live
  // session both come back from conversations/<id>.sqlite block rows.
  // The connection row (and its encrypted key) came back from control.sqlite.
  const restarted = await createBackend({ dataDir, port: 0, providerTypes: types() })
  const cold = await api<{ blocks: Block[] }>(restarted, `/api/conversations/${conversationId}/transcript`)
  expect(cold.blocks.map((block) => block.id)).toEqual(before.map((block) => block.id))

  const { client: resumed } = await connectClient(restarted, conversationId)
  await resumed.open(selection, '/ignored', 'ignored')
  await waitFor(() => resumed.transcript().blocks.length === before.length)
  expect(resumed.transcript().blocks.map((block) => block.id)).toEqual(before.map((block) => block.id))

  // The restored session keeps working and appends new rows.
  await resumed.send([{ type: 'text', text: 'and again' }])
  expect(resumed.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'response',
    'user',
    'text',
    'response',
  ])
  await resumed.close()
  await restarted.close()
}, 30_000)

test('a malformed PATCH body is rejected with 400 invalid_body', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-backend-badbody-'))
  const backend = await createBackend({ dataDir, port: 0 })
  const created = await api<{ conversation: { id: string } }>(backend, '/api/conversations', { method: 'POST' })

  const bad = await fetch(`${backend.url}/api/conversations/${created.conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ archived: 'yes' }),
    headers: { 'content-type': 'application/json' },
  })
  expect(bad.status).toBe(400)
  expect(await bad.json()).toMatchObject({ code: 'invalid_body' })

  const notJson = await fetch(`${backend.url}/api/conversations/${created.conversation.id}`, {
    method: 'PATCH',
    body: 'not json',
    headers: { 'content-type': 'application/json' },
  })
  expect(notJson.status).toBe(400)
  await backend.close()
})
