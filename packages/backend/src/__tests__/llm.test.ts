import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport } from '@demicodes/agent'
import { defineProvider, type AgentProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { decodeUtf8, deferred, delay, waitFor } from '@demicodes/utils'
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

async function openConversation(backend: TestBackend, providerId: string, conversationId?: string) {
  const id = conversationId ?? (await api<{ conversation: { id: string } }>(backend, '/api/conversations', { method: 'POST' })).body.conversation.id
  const socket = backend.session.socket(`/api/conversations/${id}/stream`)
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

test('subscription login: concurrent completions publish one provider with its vault pool', async () => {
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
  const competing = await api<{ login: { id: string } }>(backend, '/api/providers/subscription-login', post({ providerType: 'stub-sub', label: 'Competing' }))
  expect(competing.status).toBe(202)

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
  let states: Array<{ status: string; providerId?: string; message?: string }> = []
  await waitFor(() => {
    void Promise.all([loginId, competing.body.login.id].map((id) => api<{ login: (typeof states)[number] }>(backend, `/api/providers/subscription-login/${id}`))).then((polled) => {
      states = polled.map((result) => result.body.login)
    })
    return states.length === 2 && states.every((state) => state.status !== 'pending')
  }, undefined, { timeoutMs: 5_000 })

  expect(states.filter((state) => state.status === 'completed')).toHaveLength(1)
  expect(states.find((state) => state.status === 'failed')?.message).toContain('already has a stub-sub subscription')
  const providerId = states.find((state) => state.status === 'completed')!.providerId!
  const listed = await api<{ providers: Array<Record<string, unknown>> }>(backend, '/api/providers')
  expect(listed.body.providers).toEqual([
    expect.objectContaining({ id: providerId, kind: 'subscription', providerType: 'stub-sub' }),
  ])
  // The login's pool became the provider's vault directory.
  expect(existsSync(join(dataDir, 'vault', providerId, 'oauth.json'))).toBe(true)
  expect(await readdir(join(dataDir, 'vault'))).toEqual([providerId])

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

test('provider edits apply after the active request, unchanged requests retain state, and deletion blocks inference', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-provider-refresh-'))
  const entered = deferred<void>()
  const release = deferred<void>()
  const calls: Array<{ key: string; request: number }> = []
  let runtimes = 0
  const backend = await openBackend({
    dataDir,
    port: 0,
    providerTypes: {
      changing: {
        credential: 'api_key',
        create: ({ providerId, label, config }) => defineProvider({
          id: providerId,
          displayName: label,
          createRuntime: () => {
            runtimes += 1
            const makeRuntime = (cursor = 0): AgentProvider => ({
              async *run() {
                if (config.kind !== 'api_key') throw new Error('Expected API-key fixture')
                calls.push({ key: config.apiKey, request: ++cursor })
                if (calls.length === 1) {
                  entered.resolve()
                  await release.promise
                }
                yield events.text(`${config.apiKey}:${cursor}`)
                yield events.response({ inputTokens: 1 })
              },
              clone: () => makeRuntime(cursor),
            })
            return makeRuntime()
          },
        }),
      },
    },
  })
  const created = await api<{ provider: { id: string } }>(backend, '/api/providers', post({ providerType: 'changing', label: 'Changing', apiKey: 'old-key' }))
  const providerId = created.body.provider.id
  const client = await openConversation(backend, providerId)
  try {
    const first = client.send([{ type: 'text', text: 'before edit' }])
    await entered.promise
    expect((await api(backend, `/api/providers/${providerId}`, patch({ apiKey: 'new-key' }))).status).toBe(200)
    release.resolve()
    await first
    await client.send([{ type: 'text', text: 'after edit' }])
    await client.send([{ type: 'text', text: 'same configuration' }])
    expect(runtimes).toBe(2)
    expect(calls).toEqual([{ key: 'old-key', request: 1 }, { key: 'new-key', request: 1 }, { key: 'new-key', request: 2 }])

    expect((await backend.session.fetch(`/api/providers/${providerId}`, { method: 'DELETE' })).status).toBe(204)
    const errors: string[] = []
    client.subscribe((event) => { if (event.type === 'error') errors.push(event.message) })
    await client.send([{ type: 'text', text: 'after deletion' }]).catch(() => {})
    expect(errors.some((message) => message.includes('no longer available'))).toBe(true)
    expect(calls).toHaveLength(3)
    const usage = await api<{ totals: Array<{ requests: number }> }>(backend, '/api/usage')
    expect(usage.body.totals[0]?.requests).toBe(3)
  } finally {
    release.resolve()
    await client.close()
    await backend.close()
  }
}, 15_000)

test('a process provider reuses its process on one target and replaces it after a workspace switch', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-provider-target-'))
  const observed: string[] = []
  let runtimes = 0
  const backend = await openBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      process: {
        credential: 'api_key',
        create: ({ providerId, label, session }) => defineProvider({
          id: providerId,
          displayName: label,
          requiresProcessCapableHost: true,
          createRuntime: () => {
            runtimes += 1
            const makeRuntime = (): AgentProvider => {
              let processHome: string | undefined
              return {
                async *run() {
                  if (processHome === undefined) {
                    const handle = await session!.spawn({ command: '/bin/sh', args: ['-c', 'printf "$HOME"'], cwd: '/tmp' })
                    processHome = ''
                    for await (const chunk of handle.stdout) processHome += decodeUtf8(chunk)
                    expect((await handle.wait()).exitCode).toBe(0)
                  }
                  observed.push(processHome)
                  yield events.text(processHome)
                  yield events.response()
                },
                clone: makeRuntime,
              }
            }
            return makeRuntime()
          },
        }),
      },
    },
  })
  const runners: Awaited<ReturnType<typeof startTinyjsRunner>>[] = []
  let client: AgentClient | undefined
  try {
    const workspaces: Array<{ id: string; path: string }> = []
    for (const name of ['a', 'b']) {
      const home = await mkdtemp(join(tmpdir(), `demi-provider-${name}-`))
      const stateDir = await mkdtemp(join(tmpdir(), 'demi-provider-runner-'))
      const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home, name })
      runners.push(runner)
      await waitFor(() => runner.codes.length > 0, () => runner.log.join('\n'), { timeoutMs: 10_000 })
      const claimed = await api<{ device: { id: string } }>(backend, '/api/devices/claim', post({ code: runner.codes[0] }))
      const workspace = await api<{ workspace: { id: string; path: string } }>(backend, '/api/workspaces', post({ deviceId: claimed.body.device.id, path: home, name }))
      workspaces.push(workspace.body.workspace)
    }
    const created = await api<{ provider: { id: string } }>(backend, '/api/providers', post({ providerType: 'process', label: 'Process', apiKey: 'fake-key' }))
    const conversation = await api<{ conversation: { id: string } }>(backend, '/api/conversations', { method: 'POST' })
    const path = `/api/conversations/${conversation.body.conversation.id}`
    expect((await api(backend, path, patch({ workspaceId: workspaces[0]!.id }))).status).toBe(200)
    client = await openConversation(backend, created.body.provider.id, conversation.body.conversation.id)
    await client.send([{ type: 'text', text: 'first target' }])
    await client.send([{ type: 'text', text: 'same target' }])
    expect(runtimes).toBe(1)
    expect((await api(backend, path, patch({ workspaceId: workspaces[1]!.id }))).status).toBe(200)
    await client.send([{ type: 'text', text: 'second target' }])
    expect(observed).toEqual([workspaces[0]!.path, workspaces[0]!.path, workspaces[1]!.path])
    expect(runtimes).toBe(2)
  } finally {
    await client?.close()
    for (const runner of runners) await runner.stop()
    await backend.close()
  }
}, 30_000)

test('a DeepSeek vendor tool continuation replays reasoning to the compatible endpoint', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-deepseek-replay-'))
  const requests: Array<{ messages: Array<{ role: string; reasoning_content?: string; tool_calls?: unknown[] }> }> = []
  const upstream = Bun.serve({
    port: 0,
    async fetch(request) {
      const body = await request.json() as (typeof requests)[number]
      requests.push(body)
      if (requests.length > 1) {
        const assistant = body.messages.find((message) => message.tool_calls?.length)
        if (assistant?.reasoning_content !== 'Read the current directory.') {
          return Response.json({ error: { message: 'Missing reasoning_content in assistant tool-call message' } }, { status: 400 })
        }
      }
      const delta = requests.length === 1
        ? { reasoning_content: 'Read the current directory.', tool_calls: [{ index: 0, id: 'call-1', function: { name: 'shell_exec', arguments: JSON.stringify({ script: 'pwd', timeoutMs: 1000 }) } }] }
        : { content: 'done' }
      return new Response(`data: ${JSON.stringify({ choices: [{ delta }] })}\n\ndata: [DONE]\n\n`, { headers: { 'content-type': 'text/event-stream' } })
    },
  })
  const backend = await openBackend({ dataDir, port: 0 })
  let client: AgentClient | undefined
  try {
    const created = await api<{ provider: { id: string } }>(backend, '/api/providers', post({ vendorId: 'deepseek', label: 'DeepSeek', apiKey: 'fake-key', baseUrl: upstream.url.toString(), modelIds: ['test-model'] }))
    client = await openConversation(backend, created.body.provider.id)
    const errors: string[] = []
    client.subscribe((event) => { if (event.type === 'error') errors.push(event.message) })
    await client.send([{ type: 'text', text: 'read the current directory' }])
    expect(errors).toEqual([])
    expect(requests).toHaveLength(2)
    expect(requests[1]?.messages.find((message) => message.tool_calls?.length)?.reasoning_content).toBe('Read the current directory.')
  } finally {
    await client?.close()
    await backend.close()
    upstream.stop(true)
  }
}, 15_000)
