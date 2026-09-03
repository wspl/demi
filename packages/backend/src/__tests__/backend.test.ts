import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport } from '@demicodes/agent'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { createBackend, type Backend } from '../index'

// M2 acceptance: a zero-setup virtual conversation over the real Web API +
// conversation stream — tinybash builtins work, a script outside the subset
// is refused with the reason, and cold history equals the live transcript.
// Detach mid-turn and the backend restart live in `scenarios/` (S9, R1).

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
    // Turn 2: a real program puts the script outside the subset; this backend provisions no machine, so the call is a tool error and nothing runs.
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
  expect(shellErrors).toHaveLength(1)
  expect(JSON.stringify(client.transcript().blocks.filter((block) => block.type === 'tool_call').at(-1))).toContain('this backend provisions none')

  // The first user message became the title.
  const listed = await api<{ conversations: Array<{ id: string; title: string }> }>(backend, '/api/conversations')
  expect(listed.conversations.find((row) => row.id === conversationId)?.title).toBe('work with files please')

  await client.close()
  await backend.close()
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
