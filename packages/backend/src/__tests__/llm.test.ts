import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport } from '@demicodes/agent'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { deferred, delay, waitFor } from '@demicodes/utils'
import { startTinyjsRunner } from '@demicodes/runner/testing'
import type { SessionProviderContext } from '../llm/assembly'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { openBackend, type TestBackend } from './session'

// M5 step 1 (BYOK + metering): a pasted key becomes a usable connection —
// providers assemble per connection from vault credentials, every provider
// request lands in the usage ledger, and the rate limit refuses at the
// inference entry.

function selectionFor(connectionId: string) {
  const model: ModelSelection = {
    providerId: connectionId,
    model: { id: 'test-model', name: 'M', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
    thinking: null,
  }
  return { providerId: connectionId, model }
}

async function api<T>(backend: TestBackend, path: string, init?: RequestInit): Promise<{ status: number; body: T }> {
  const response = await backend.session.fetch(path, init)
  return { status: response.status, body: (await response.json().catch(() => null)) as T }
}

function post(payload: unknown): RequestInit {
  return { method: 'POST', body: JSON.stringify(payload), headers: { 'content-type': 'application/json' } }
}

function stubBackendOptions(dataDir: string, turns: number): Parameters<typeof openBackend>[0] {
  return {
    dataDir,
    port: 0,
    providerTypes: {
      stub: ({ connectionId, label }) =>
        defineProvider({
          id: connectionId,
          displayName: label,
          createRuntime: () =>
            new StubProvider(
              Array.from({ length: turns }, (_, index) => [
                events.text(`answer ${index}`),
                events.response({ inputTokens: 100 + index, outputTokens: 10 }),
              ]),
            ),
        }),
    },
  }
}

async function openConversation(backend: TestBackend, connectionId: string) {
  const created = await api<{ conversation: { id: string } }>(backend, '/api/conversations', { method: 'POST' })
  const socket = backend.session.socket(`/api/conversations/${created.body.conversation.id}/stream`)
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener('open', () => resolve(), { once: true })
    socket.addEventListener('error', () => reject(new Error('stream connect failed')), { once: true })
  })
  const client = new AgentClient(createWebSocketClientTransport(socket as never))
  await client.open(selectionFor(connectionId), '/ignored', 'ignored')
  return client
}

test('connections: create/list redact key material, unknown types rejected, delete unresolves', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m5-conn-'))
  const backend = await openBackend(stubBackendOptions(dataDir, 1))

  const unknown = await api(backend, '/api/connections', post({ type: 'nope', label: 'x', apiKey: 'k' }))
  expect(unknown.status).toBe(400)

  const created = await api<{ connection: Record<string, unknown> }>(
    backend,
    '/api/connections',
    post({ type: 'stub', label: 'My Stub', apiKey: 'sk-super-secret', modelIds: ['custom-1', 'custom-2'] }),
  )
  expect(created.status).toBe(201)
  expect(JSON.stringify(created.body)).not.toContain('sk-super-secret')
  const connectionId = created.body.connection.id as string

  const listed = await api<{ connections: Array<Record<string, unknown>> }>(backend, '/api/connections')
  expect(listed.body.connections).toEqual([
    expect.objectContaining({ id: connectionId, type: 'stub', label: 'My Stub', modelIds: ['custom-1', 'custom-2'] }),
  ])
  expect(JSON.stringify(listed.body)).not.toContain('sk-super-secret')

  // The catalog groups by connection; user-entered model ids become entries.
  const models = await api<{ connections: Array<{ connectionId: string; models: Array<{ id: string; providerId: string }> }> }>(
    backend,
    '/api/models',
  )
  expect(models.body.connections).toHaveLength(1)
  expect(models.body.connections[0]?.models.map((model) => model.id)).toEqual(['custom-1', 'custom-2'])
  expect(models.body.connections[0]?.models[0]?.providerId).toBe(connectionId)

  const deleted = await backend.session.fetch(`/api/connections/${connectionId}`, { method: 'DELETE' })
  expect(deleted.status).toBe(204)
  expect((await api<{ connections: unknown[] }>(backend, '/api/connections')).body.connections).toHaveLength(0)
  expect((await api<{ connections: unknown[] }>(backend, '/api/models')).body.connections).toHaveLength(0)

  await backend.close()
}, 15_000)

test('metering: every provider request lands in the ledger; /api/usage aggregates', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m5-usage-'))
  const backend = await openBackend(stubBackendOptions(dataDir, 3))
  const created = await api<{ connection: { id: string } }>(
    backend,
    '/api/connections',
    post({ type: 'stub', label: 'Stub', apiKey: 'k' }),
  )
  const connectionId = created.body.connection.id

  const client = await openConversation(backend, connectionId)
  await client.send([{ type: 'text', text: 'one' }])
  await client.send([{ type: 'text', text: 'two' }])

  // Ledger appends are fire-and-forget off the event stream; give them a beat.
  await delay(50)
  const usage = await api<{
    totals: Array<{ connectionId: string; modelId: string; requests: number; inputTokens: number; outputTokens: number }>
  }>(backend, '/api/usage')
  expect(usage.body.totals).toEqual([
    expect.objectContaining({
      connectionId,
      modelId: 'test-model',
      requests: 2,
      inputTokens: 201, // 100 + 101
      outputTokens: 20,
    }),
  ])

  await client.close()
  await backend.close()
}, 15_000)

test('enforcement: the provider request rate limit refuses at the inference entry', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m5-limit-'))
  const backend = await openBackend({ ...stubBackendOptions(dataDir, 5), usage: { providerRequestsPerMinute: 1 } })
  const created = await api<{ connection: { id: string } }>(
    backend,
    '/api/connections',
    post({ type: 'stub', label: 'Stub', apiKey: 'k' }),
  )
  const client = await openConversation(backend, created.body.connection.id)
  const errors: string[] = []
  client.subscribe((event) => {
    if (event.type === 'error') errors.push(event.message)
  })

  await client.send([{ type: 'text', text: 'allowed' }])
  await client.send([{ type: 'text', text: 'refused' }]).catch(() => {})
  await waitFor(() => errors.length > 0, undefined, { timeoutMs: 5_000 })
  expect(errors[0]).toContain('rate limit')

  await client.close()
  await backend.close()
}, 15_000)

test('subscription login: pending material surfaces, completion becomes a connection with a vault pool', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m5-sub-'))
  const approve = deferred<void>()
  const backend = await openBackend({
    dataDir,
    port: 0,
    providerTypes: {
      'stub-sub': ({ connectionId, label, vaultDir }) =>
        defineProvider({
          id: connectionId,
          displayName: label,
          credentials: {
            capability: () => ({ mode: 'supported', canBeginLogin: true }),
            list: () => [],
            getActive: () => ({ credentialId: 'cred-1', status: { status: 'ok' } as never }),
            setActive: () => ({ credentialId: 'cred-1', status: { status: 'ok' } as never }),
            beginLogin: async (options) => {
              options?.onPending?.({ verificationUrl: 'https://verify.example/device', userCode: 'ABCD-1234' })
              await approve.promise
              await mkdir(vaultDir, { recursive: true })
              await writeFile(join(vaultDir, 'oauth.json'), '{"token":"secret"}')
              return { status: 'completed', credentialId: 'cred-1' }
            },
          },
          createRuntime: () => new StubProvider([[events.text('hello'), events.response()]]),
        }),
    },
  })

  // A type without a native login flow is refused.
  const noFlow = await api(backend, '/api/connections/subscription-login', post({ type: 'anthropic' }))
  expect(noFlow.status).toBe(400)

  const started = await api<{ login: { id: string } }>(
    backend,
    '/api/connections/subscription-login',
    post({ type: 'stub-sub', label: 'My Subscription' }),
  )
  expect(started.status).toBe(202)
  const loginId = started.body.login.id

  await waitFor(() => false, undefined, { timeoutMs: 30, intervalMs: 10 }).catch(() => {})
  const pending = await api<{ login: { status: string; verificationUrl: string; userCode: string } }>(
    backend,
    `/api/connections/subscription-login/${loginId}`,
  )
  expect(pending.body.login).toEqual({
    status: 'pending',
    verificationUrl: 'https://verify.example/device',
    userCode: 'ABCD-1234',
  })

  approve.resolve()
  let state: { status: string; connectionId?: string } = { status: 'pending' }
  await waitFor(() => {
    void api<{ login: typeof state }>(backend, `/api/connections/subscription-login/${loginId}`).then((polled) => {
      state = polled.body.login
    })
    return state.status === 'completed'
  }, undefined, { timeoutMs: 5_000 })

  const connectionId = state.connectionId as string
  const listed = await api<{ connections: Array<Record<string, unknown>> }>(backend, '/api/connections')
  expect(listed.body.connections).toEqual([
    expect.objectContaining({ id: connectionId, kind: 'subscription', type: 'stub-sub', label: 'My Subscription' }),
  ])
  // The login's pool became the connection's vault directory.
  expect(existsSync(join(dataDir, 'vault', connectionId, 'oauth.json'))).toBe(true)

  const deleted = await backend.session.fetch(`/api/connections/${connectionId}`, { method: 'DELETE' })
  expect(deleted.status).toBe(204)
  expect(existsSync(join(dataDir, 'vault', connectionId))).toBe(false)

  await backend.close()
}, 15_000)

test('a process-capable provider gets a session-scoped instance carrying the target spawn', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m5-cli-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m5-cli-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m5-cli-runner-'))
  const sessions: SessionProviderContext[] = []
  const backend = await openBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      'stub-cli': ({ connectionId, label, session }) => {
        if (session) sessions.push(session)
        return defineProvider({
          id: connectionId,
          displayName: label,
          requiresProcessCapableHost: true,
          createRuntime: () => new StubProvider([[events.text('cli turn'), events.response()]]),
        })
      },
    },
  })

  // Claim a runner and bind a conversation's workspace to it (M4 machinery).
  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'cli-device' })
  await waitFor(() => runner.codes.length > 0, () => runner.log.join('\n'), { timeoutMs: 10_000 })
  const codes = runner.codes
  const claimed = await api<{ device: { id: string } }>(backend, '/api/devices/claim', post({ code: codes[0] }))
  const created = await api<{ connection: { id: string } }>(
    backend,
    '/api/connections',
    post({ type: 'stub-cli', label: 'CLI', apiKey: 'k' }),
  )
  const connectionId = created.body.connection.id
  const conversation = await api<{ conversation: { id: string } }>(backend, '/api/conversations', { method: 'POST' })
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control = new LocalControlService(controlDb)
  const workspace = await control.createWorkspace({
    userId: backend.session.user.id,
    deviceId: claimed.body.device.id,
    path: runnerDir,
    name: 'cli workspace',
  })
  await control.setConversationWorkspace(conversation.body.conversation.id, workspace.id)
  controlDb.close()

  const socket = backend.session.socket(`/api/conversations/${conversation.body.conversation.id}/stream`)
  await new Promise<void>((resolve) => socket.addEventListener('open', () => resolve(), { once: true }))
  const client = new AgentClient(createWebSocketClientTransport(socket as never))
  await client.open(selectionFor(connectionId), '/ignored', 'ignored')

  expect(sessions).toHaveLength(1)
  const session = sessions[0]!

  // The session spawn executes on the claimed device.
  const handle = await session.spawn({ command: 'touch', args: ['spawned.marker'], cwd: runnerDir })
  await handle.wait()
  expect(existsSync(join(runnerDir, 'spawned.marker'))).toBe(true)

  await client.close()
  await runner.stop()
  await backend.close()
}, 20_000)
