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

// M5 step 1 (BYOK + metering): a pasted key becomes a usable provider —
// providers assemble per provider from vault credentials, every provider
// request lands in the usage ledger, and the rate limit refuses at the
// inference entry.

function selectionFor(providerId: string) {
  const model: ModelSelection = {
    providerId: providerId,
    model: { id: 'test-model', name: 'M', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
    thinking: null,
  }
  return { providerId: providerId, model }
}

async function api<T>(backend: TestBackend, path: string, init?: RequestInit): Promise<{ status: number; body: T }> {
  const response = await backend.session.fetch(path, init)
  return { status: response.status, body: (await response.json().catch(() => null)) as T }
}

function post(payload: unknown): RequestInit {
  return { method: 'POST', body: JSON.stringify(payload), headers: { 'content-type': 'application/json' } }
}

function patch(payload: unknown): RequestInit {
  return { method: 'PATCH', body: JSON.stringify(payload), headers: { 'content-type': 'application/json' } }
}

function stubBackendOptions(dataDir: string, turns: number): Parameters<typeof openBackend>[0] {
  return {
    dataDir,
    port: 0,
    providerTypes: {
      stub: {
        credential: 'api_key',
        create: ({ providerId, label }) =>
          defineProvider({
            id: providerId,
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
    },
  }
}

async function openConversation(backend: TestBackend, providerId: string) {
  const created = await api<{ conversation: { id: string } }>(backend, '/api/conversations', { method: 'POST' })
  const socket = backend.session.socket(`/api/conversations/${created.body.conversation.id}/stream`)
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener('open', () => resolve(), { once: true })
    socket.addEventListener('error', () => reject(new Error('stream connect failed')), { once: true })
  })
  const client = new AgentClient(createWebSocketClientTransport(socket as never))
  await client.open(selectionFor(providerId), '/ignored', 'ignored')
  return client
}

test('providers: create/list redact key material, unknown types rejected, delete unresolves', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m5-conn-'))
  const backend = await openBackend(stubBackendOptions(dataDir, 1))

  const unknown = await api(backend, '/api/providers', post({ providerType: 'nope', label: 'x', apiKey: 'k' }))
  expect(unknown.status).toBe(400)

  const created = await api<{ provider: Record<string, unknown> }>(
    backend,
    '/api/providers',
    post({ providerType: 'stub', label: 'My Stub', apiKey: 'sk-super-secret', modelIds: ['custom-1', 'custom-2'] }),
  )
  expect(created.status).toBe(201)
  expect(JSON.stringify(created.body)).not.toContain('sk-super-secret')
  const providerId = created.body.provider.id as string

  const listed = await api<{ providers: Array<Record<string, unknown>> }>(backend, '/api/providers')
  expect(listed.body.providers).toEqual([
    expect.objectContaining({ id: providerId, providerType: 'stub', label: 'My Stub', modelIds: ['custom-1', 'custom-2'] }),
  ])
  expect(JSON.stringify(listed.body)).not.toContain('sk-super-secret')

  // The catalog groups by provider; user-entered model ids become entries.
  const models = await api<{ providers: Array<{ providerId: string; models: Array<{ id: string; providerId: string }> }> }>(
    backend,
    '/api/models',
  )
  expect(models.body.providers).toHaveLength(1)
  expect(models.body.providers[0]?.models.map((model) => model.id)).toEqual(['custom-1', 'custom-2'])
  expect(models.body.providers[0]?.models[0]?.providerId).toBe(providerId)

  const deleted = await backend.session.fetch(`/api/providers/${providerId}`, { method: 'DELETE' })
  expect(deleted.status).toBe(204)
  expect((await api<{ providers: unknown[] }>(backend, '/api/providers')).body.providers).toHaveLength(0)
  expect((await api<{ providers: unknown[] }>(backend, '/api/models')).body.providers).toHaveLength(0)

  await backend.close()
}, 15_000)

test('metering: every provider request lands in the ledger; /api/usage aggregates', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m5-usage-'))
  const backend = await openBackend(stubBackendOptions(dataDir, 3))
  const created = await api<{ provider: { id: string } }>(
    backend,
    '/api/providers',
    post({ providerType: 'stub', label: 'Stub', apiKey: 'k' }),
  )
  const providerId = created.body.provider.id

  const client = await openConversation(backend, providerId)
  await client.send([{ type: 'text', text: 'one' }])
  await client.send([{ type: 'text', text: 'two' }])

  // Ledger appends are fire-and-forget off the event stream; give them a beat.
  await delay(50)
  const usage = await api<{
    totals: Array<{ providerId: string; modelId: string; requests: number; inputTokens: number; outputTokens: number }>
  }>(backend, '/api/usage')
  expect(usage.body.totals).toEqual([
    expect.objectContaining({
      providerId,
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
  const created = await api<{ provider: { id: string } }>(
    backend,
    '/api/providers',
    post({ providerType: 'stub', label: 'Stub', apiKey: 'k' }),
  )
  const client = await openConversation(backend, created.body.provider.id)
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

test('subscription login: pending material surfaces, completion becomes a provider with a vault pool', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m5-sub-'))
  const approve = deferred<void>()
  const backend = await openBackend({
    dataDir,
    port: 0,
    providerTypes: {
      'stub-sub': {
        credential: 'subscription',
        create: ({ providerId, label, vaultDir }) =>
        defineProvider({
          id: providerId,
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
    },
  })

  // A type without a native login flow is refused.
  const noFlow = await api(backend, '/api/providers/subscription-login', post({ type: 'anthropic' }))
  expect(noFlow.status).toBe(400)

  const started = await api<{ login: { id: string } }>(
    backend,
    '/api/providers/subscription-login',
    post({ providerType: 'stub-sub', label: 'My Subscription' }),
  )
  expect(started.status).toBe(202)
  const loginId = started.body.login.id

  await waitFor(() => false, undefined, { timeoutMs: 30, intervalMs: 10 }).catch(() => {})
  const pending = await api<{ login: { status: string; verificationUrl: string; userCode: string } }>(
    backend,
    `/api/providers/subscription-login/${loginId}`,
  )
  expect(pending.body.login).toEqual({
    status: 'pending',
    verificationUrl: 'https://verify.example/device',
    userCode: 'ABCD-1234',
  })

  approve.resolve()
  let state: { status: string; providerId?: string } = { status: 'pending' }
  await waitFor(() => {
    void api<{ login: typeof state }>(backend, `/api/providers/subscription-login/${loginId}`).then((polled) => {
      state = polled.body.login
    })
    return state.status === 'completed'
  }, undefined, { timeoutMs: 5_000 })

  const providerId = state.providerId as string
  const listed = await api<{ providers: Array<Record<string, unknown>> }>(backend, '/api/providers')
  expect(listed.body.providers).toEqual([
    expect.objectContaining({ id: providerId, kind: 'subscription', providerType: 'stub-sub', label: 'My Subscription' }),
  ])
  // The login's pool became the provider's vault directory.
  expect(existsSync(join(dataDir, 'vault', providerId, 'oauth.json'))).toBe(true)

  // One subscription per family per scope: the catalog says it is configured, a second login is refused.
  const catalog = await api<{ subscriptions: Array<{ providerType: string; configured: boolean }> }>(backend, '/api/providers/catalog')
  expect(catalog.body.subscriptions).toContainEqual({ providerType: 'stub-sub', configured: true })
  const again = await api<{ code: string }>(backend, '/api/providers/subscription-login', post({ providerType: 'stub-sub' }))
  expect(again.status).toBe(409)
  expect(again.body.code).toBe('provider_exists')
  // A subscription entry takes a new label and nothing else.
  const relabel = await api<{ provider: { label: string } }>(backend, `/api/providers/${providerId}`, patch({ label: 'Work' }))
  expect(relabel.body.provider.label).toBe('Work')
  expect((await api(backend, `/api/providers/${providerId}`, patch({ apiKey: 'k' }))).status).toBe(400)

  const deleted = await backend.session.fetch(`/api/providers/${providerId}`, { method: 'DELETE' })
  expect(deleted.status).toBe(204)
  expect(existsSync(join(dataDir, 'vault', providerId))).toBe(false)

  await backend.close()
}, 15_000)

test('the vendor catalog: entries from models.dev vendors carry the family and a live model list; custom endpoints name theirs', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-vendors-'))
  const backend = await openBackend(stubBackendOptions(dataDir, 1))

  // Only vendors whose protocol one of our runtimes speaks are offered; Copilot is excluded by name.
  const catalog = await api<{ vendors: Array<Record<string, unknown>> }>(backend, '/api/providers/catalog')
  expect(catalog.status).toBe(200)
  expect(catalog.body.vendors).toEqual([
    { id: 'deepseek', name: 'DeepSeek', providerType: 'openai', wireApi: 'chat-completions', baseUrl: 'https://api.deepseek.com', doc: 'https://api-docs.deepseek.com' },
    { id: 'minimax', name: 'MiniMax', providerType: 'anthropic', baseUrl: 'https://api.minimax.io/anthropic/v1', doc: null },
  ])

  expect((await api<{ code: string }>(backend, '/api/providers', post({ vendorId: 'amazon-bedrock', label: 'x', apiKey: 'k' }))).body.code).toBe('unknown_vendor')

  // A vendor entry: the family, protocol and endpoint come from the vendor; the model list is the vendor's, live.
  const created = await api<{ provider: Record<string, unknown> }>(backend, '/api/providers', post({ vendorId: 'deepseek', label: 'DeepSeek', apiKey: 'sk-1' }))
  expect(created.status).toBe(201)
  expect(created.body.provider).toEqual(
    expect.objectContaining({ providerType: 'openai', wireApi: 'chat-completions', vendorId: 'deepseek', baseUrl: 'https://api.deepseek.com', modelIds: null }),
  )
  const providerId = created.body.provider.id as string
  type Catalog = { providers: Array<{ providerId: string; models: Array<Record<string, unknown>> }> }
  const live = (await api<Catalog>(backend, '/api/models')).body.providers.find((p) => p.providerId === providerId)!
  expect(live.models.map((m) => m.id)).toEqual(['deepseek-v4', 'deepseek-v4-flash'])
  expect(live.models[0]).toEqual(
    expect.objectContaining({ providerId, displayName: 'DeepSeek V4', contextWindow: 128_000, outputLimit: 32_000, supportsTools: true, supportsReasoning: true }),
  )

  // Editing: a typed model list replaces the live one; null returns to it; the endpoint and key can change.
  const typed = await api<{ provider: Record<string, unknown> }>(backend, `/api/providers/${providerId}`, patch({ label: 'DS', modelIds: ['deepseek-v4'], baseUrl: 'https://proxy.example/v1', apiKey: 'sk-2' }))
  expect(typed.body.provider).toEqual(expect.objectContaining({ label: 'DS', modelIds: ['deepseek-v4'], baseUrl: 'https://proxy.example/v1', vendorId: 'deepseek' }))
  expect(JSON.stringify(typed.body)).not.toContain('sk-2')
  const typedCatalog = (await api<Catalog>(backend, '/api/models')).body.providers.find((p) => p.providerId === providerId)!
  expect(typedCatalog.models.map((m) => m.id)).toEqual(['deepseek-v4'])
  await api(backend, `/api/providers/${providerId}`, patch({ modelIds: null }))
  const liveAgain = (await api<Catalog>(backend, '/api/models')).body.providers.find((p) => p.providerId === providerId)!
  expect(liveAgain.models.map((m) => m.id)).toEqual(['deepseek-v4', 'deepseek-v4-flash'])

  // A custom endpoint names its family and protocol itself; subscription families are not created this way.
  const custom = await api<{ provider: Record<string, unknown> }>(
    backend,
    '/api/providers',
    post({ providerType: 'openai', wireApi: 'chat-completions', label: 'Gateway', apiKey: 'k', baseUrl: 'https://gw.example/v1', modelIds: ['internal-70b'] }),
  )
  expect(custom.status).toBe(201)
  expect(custom.body.provider).toEqual(expect.objectContaining({ providerType: 'openai', wireApi: 'chat-completions', vendorId: null, modelIds: ['internal-70b'] }))
  expect((await api<{ code: string }>(backend, '/api/providers', post({ providerType: 'claude-code', label: 'x', apiKey: 'k' }))).body.code).toBe('subscription_only')

  await backend.close()
})

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
      'stub-cli': {
        credential: 'api_key',
        create: ({ providerId, label, session }) => {
          if (session) sessions.push(session)
          return defineProvider({
            id: providerId,
            displayName: label,
            requiresProcessCapableHost: true,
            createRuntime: () => new StubProvider([[events.text('cli turn'), events.response()]]),
          })
        },
      },
    },
  })

  // Claim a runner and bind a conversation's workspace to it (M4 machinery).
  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'cli-device' })
  await waitFor(() => runner.codes.length > 0, () => runner.log.join('\n'), { timeoutMs: 10_000 })
  const codes = runner.codes
  const claimed = await api<{ device: { id: string } }>(backend, '/api/devices/claim', post({ code: codes[0] }))
  const created = await api<{ provider: { id: string } }>(
    backend,
    '/api/providers',
    post({ providerType: 'stub-cli', label: 'CLI', apiKey: 'k' }),
  )
  const providerId = created.body.provider.id
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
  await client.open(selectionFor(providerId), '/ignored', 'ignored')

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
