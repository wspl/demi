import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport } from '@demicodes/agent'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { delay } from '@demicodes/utils'
import { createBackend } from '../index'
import { login, openBackend, type TestBackend, type WebSession } from './session'

// M12 checkpoint 3: the instance mode — who configures providers, whose
// connections a caller sees and may select, the shared-mode instance
// ledger, and the mode fixed once providers exist.

const json = (body: unknown, method = 'POST'): RequestInit => ({ method, body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })

function stubOptions(dataDir: string, mode: 'shared' | 'isolated'): Parameters<typeof openBackend>[0] {
  return {
    dataDir,
    port: 0,
    mode,
    providerTypes: {
      stub: ({ connectionId, label }) =>
        defineProvider({
          id: connectionId,
          displayName: label,
          createRuntime: () => new StubProvider([[events.text('hi'), events.response({ inputTokens: 10, outputTokens: 1 })]]),
        }),
    },
  }
}

async function createConnection(actor: WebSession, label: string) {
  const response = await actor.fetch('/api/connections', json({ type: 'stub', label, apiKey: 'k' }))
  return { status: response.status, id: ((await response.json().catch(() => null)) as { connection?: { id: string } } | null)?.connection?.id ?? null }
}

async function listIds(actor: WebSession, path: '/api/connections' | '/api/models'): Promise<string[]> {
  const body = (await (await actor.fetch(path)).json()) as { connections: Array<{ id?: string; connectionId?: string }> }
  return body.connections.map((entry) => entry.id ?? entry.connectionId!)
}

async function createUser(backend: TestBackend, username: string): Promise<WebSession> {
  const response = await backend.session.fetch('/api/users', json({ username, password: `${username}-pass-1`, role: 'user' }))
  if (response.status !== 201) throw new Error(`create ${username}: HTTP ${response.status}`)
  return login(backend, username, `${username}-pass-1`)
}

/** One turn on a fresh conversation over `connectionId`; resolves once the answer arrived. */
async function turn(backend: TestBackend, actor: WebSession, connectionId: string): Promise<void> {
  const created = (await (await actor.fetch('/api/conversations', { method: 'POST' })).json()) as { conversation: { id: string } }
  const socket = actor.socket(`/api/conversations/${created.conversation.id}/stream`)
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener('open', () => resolve(), { once: true })
    socket.addEventListener('error', () => reject(new Error('stream connect failed')), { once: true })
  })
  const client = new AgentClient(createWebSocketClientTransport(socket as never))
  const model: ModelSelection = {
    providerId: connectionId,
    model: { id: 'm', name: 'M', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
    thinking: null,
  }
  await client.open({ providerId: connectionId, model }, '/ignored', 'ignored')
  await client.send([{ type: 'text', text: 'go' }])
  await client.close()
  void backend
}

test('shared mode: admins configure the instance connections, everyone uses them, the admin reads the ledger by user', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-mode-shared-'))
  const backend = await openBackend(stubOptions(dataDir, 'shared'))
  const master = backend.session
  const bob = await createUser(backend, 'bob')

  expect((await createConnection(bob, 'mine')).status).toBe(403)
  const shared = await createConnection(master, 'Instance stub')
  expect(shared.status).toBe(201)
  expect(await listIds(bob, '/api/connections')).toEqual([shared.id!])
  expect(await listIds(bob, '/api/models')).toEqual([shared.id!])
  expect((await bob.fetch(`/api/connections/${shared.id}`, { method: 'DELETE' })).status).toBe(403)

  await turn(backend, bob, shared.id!)
  await turn(backend, master, shared.id!)
  await delay(50)
  const own = (await (await bob.fetch('/api/usage')).json()) as { totals: Array<{ requests: number }> }
  expect(own.totals).toEqual([expect.objectContaining({ connectionId: shared.id, requests: 1 })])

  expect((await bob.fetch('/api/usage/instance')).status).toBe(403)
  const instance = (await (await master.fetch('/api/usage/instance')).json()) as { users: Array<{ username: string; totals: Array<{ requests: number }> }> }
  expect(instance.users.map((entry) => `${entry.username}:${entry.totals[0]?.requests ?? 0}`)).toEqual(['master:1', 'bob:1'])

  await backend.close()

  // The mode is fixed once providers are configured.
  await expect(createBackend({ ...stubOptions(dataDir, 'isolated'), mode: 'isolated' })).rejects.toThrow('configured under the other instance mode')
}, 20_000)

test('isolated mode: every user configures and sees their own connections; nothing of another user is reachable', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-mode-isolated-'))
  const backend = await openBackend(stubOptions(dataDir, 'isolated'))
  const alice = await createUser(backend, 'alice')
  const bob = await createUser(backend, 'bob')

  const mine = await createConnection(alice, "alice's")
  const theirs = await createConnection(bob, "bob's")
  expect([mine.status, theirs.status]).toEqual([201, 201])
  expect(await listIds(alice, '/api/connections')).toEqual([mine.id!])
  expect(await listIds(alice, '/api/models')).toEqual([mine.id!])
  expect(await listIds(backend.session, '/api/connections')).toEqual([])
  expect((await bob.fetch(`/api/connections/${mine.id}`, { method: 'DELETE' })).status).toBe(404)
  expect((await bob.fetch(`/api/connections/${mine.id}/test`, { method: 'POST' })).status).toBe(404)
  expect((await backend.session.fetch('/api/usage/instance')).status).toBe(403)

  // The model selection names a connection in the caller's scope.
  const created = (await (await bob.fetch('/api/conversations', { method: 'POST' })).json()) as { conversation: { id: string } }
  const foreign = await bob.fetch(`/api/conversations/${created.conversation.id}`, json({ connectionId: mine.id, modelId: 'm' }, 'PATCH'))
  expect(foreign.status).toBe(404)
  expect((await bob.fetch(`/api/conversations/${created.conversation.id}`, json({ connectionId: theirs.id, modelId: 'm' }, 'PATCH'))).status).toBe(200)

  await turn(backend, alice, mine.id!)
  await delay(50)
  expect(((await (await alice.fetch('/api/usage')).json()) as { totals: unknown[] }).totals).toHaveLength(1)
  expect(((await (await bob.fetch('/api/usage')).json()) as { totals: unknown[] }).totals).toHaveLength(0)

  await backend.close()
}, 20_000)
