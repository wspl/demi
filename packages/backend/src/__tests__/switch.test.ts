import { existsSync, readFileSync } from 'node:fs'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport, type ClientSessionEvent } from '@demicodes/agent'
import { defineProvider, type AgentProvider, type ProviderEvent } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { startTinyjsRunner } from '@demicodes/runner/testing'
import { waitFor } from '@demicodes/utils'
import { LocalControlService, type ControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { createBackend, type Backend } from '../index'

// M6 acceptance, on M11's access model: target switching at turn boundaries
// over the conversations PATCH, the pending switch and its context block,
// the departed device granted and reached with `demi host shell --id`, the
// grant API (explicit grants for user devices only, revoke closes the door),
// and offline degradation.

async function api(backend: Backend, path: string, init?: RequestInit): Promise<Response> {
  return fetch(`${backend.url}${path}`, init)
}

async function json(backend: Backend, path: string, body: unknown, method = 'POST'): Promise<Response> {
  return api(backend, path, { method, body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })
}

function selectionFor(connectionId: string) {
  const model: ModelSelection = {
    providerId: connectionId,
    model: { id: 'm', name: 'M', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
    thinking: null,
  }
  return { providerId: connectionId, model }
}

async function openClient(backend: Backend, conversationId: string, selection: ReturnType<typeof selectionFor>) {
  const socket = new WebSocket(`${backend.url.replace('http', 'ws')}/api/conversations/${conversationId}/stream`)
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener('open', () => resolve(), { once: true })
    socket.addEventListener('error', () => reject(new Error('stream connect failed')), { once: true })
  })
  const client = new AgentClient(createWebSocketClientTransport(socket as never))
  const shellEvents: Extract<ClientSessionEvent, { type: 'shell_output' }>[] = []
  client.subscribe((event) => {
    if (event.type === 'shell_output') shellEvents.push(event)
  })
  await client.open(selection, '/ignored-by-server', 'ignored')
  return { client, shellEvents }
}

function lastExited(shellEvents: Extract<ClientSessionEvent, { type: 'shell_output' }>[]) {
  return shellEvents.filter((event) => event.status.status === 'exited').at(-1)?.status
}

function announcements(client: AgentClient): string[] {
  return client
    .transcript()
    .blocks.flatMap((block) => (block.type === 'user' && block.preamble?.includes('[Execution target switched]') ? [block.preamble] : []))
}

async function stubConnection(backend: Backend): Promise<string> {
  const response = await json(backend, '/api/connections', { type: 'stub', label: 'Stub', apiKey: 'test-key' })
  const { connection } = (await response.json()) as { connection: { id: string } }
  return connection.id
}

/** A stub whose turn N runs `scripts[N]` as one shell call — filled in once device ids are known. */
function scriptedStub(scripts: string[]) {
  return () =>
    new StubProvider(
      scripts.flatMap((script, index) => [
        [events.toolCall(`t${index + 1}`, 'shell_exec', { script, timeoutMs: 20_000 })],
        [events.text(`turn ${index + 1}`), events.response()],
      ]),
    )
}

test('M6 acceptance: virtual→real switch with context block, real→virtual grants the device, grant API, offline chat', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m6-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m6-runner-'))
  const scripts: string[] = []
  const backend = await createBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: ({ connectionId, label }) => defineProvider({ id: connectionId, displayName: label, createRuntime: scriptedStub(scripts) }),
    },
  })
  const selection = selectionFor(await stubConnection(backend))

  // Pair a device and create the workspace over the M6 HTTP surface.
  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'm6-device' })
  await waitFor(() => runner.codes.length > 0, undefined, { timeoutMs: 5_000 })
  const { device } = (await (await json(backend, '/api/devices/claim', { code: runner.codes[0] })).json()) as { device: { id: string } }
  const workspaceResponse = await json(backend, '/api/workspaces', { deviceId: device.id, path: runnerDir, name: 'm6 workspace' })
  expect(workspaceResponse.status).toBe(201)
  const { workspace } = (await workspaceResponse.json()) as { workspace: { id: string } }

  scripts.push(
    // Turn 1 (virtual): a file that stays behind; the status names virtual.
    'echo -n secret > notes.txt && demi host current',
    // Turn 2 (after the switch to the workspace): only the current host is reachable; a file written here is what the grant reaches later.
    'demi host list; printf secret > notes.txt && demi host current',
    // Turn 3 (after the switch back to virtual): the workspace's device was granted, its shell starts in its home.
    `demi host list && demi host shell --id ${device.id} "cat notes.txt"`,
    // Turn 4 (after the revoke): the door is closed.
    `demi host shell --id ${device.id} "cat notes.txt" || echo refused`,
    // Turn 5: chat-only while the bound workspace's runner is offline.
    'true',
  )

  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const { client, shellEvents } = await openClient(backend, conversation.id, selection)

  // Turn 1 on virtual: file exists only in the virtual fs; status names virtual.
  await client.send([{ type: 'text', text: 'write a note' }])
  expect(lastExited(shellEvents)?.stdout.delta).toContain('host: virtual')
  expect(existsSync(join(runnerDir, 'notes.txt'))).toBe(false)

  // Switch virtual→real over PATCH: the switch is pending, nothing granted (hostless has no device).
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: workspace.id }, 'PATCH')).status).toBe(200)
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control: ControlService = new LocalControlService(controlDb)
  let record = await control.getConversation(conversation.id)
  expect(record?.workspaceId).toBe(workspace.id)
  expect(record?.pendingSwitch).toEqual({
    from: { kind: 'hostless' },
    to: { kind: 'workspace', workspaceId: workspace.id, deviceId: device.id, path: runnerDir },
  })
  expect(await control.listHostGrants(conversation.id)).toEqual([])

  // Turn 2: the context block is injected once; the workspace is live.
  await client.send([{ type: 'text', text: 'bring my files' }])
  const onWorkspace = lastExited(shellEvents)
  expect(onWorkspace?.stdout.delta).toContain(`${device.id}  m6-device  online  ${runnerDir}  (current)`)
  expect(onWorkspace?.stdout.delta).toContain('host: workspace "m6 workspace"')
  expect(readFileSync(join(runnerDir, 'notes.txt'), 'utf8')).toBe('secret')
  expect(announcements(client)).toHaveLength(1)
  expect(announcements(client)[0]).toContain('Previous target: the virtual environment')
  expect(announcements(client)[0]).not.toContain('demi host shell')
  expect((await control.getConversation(conversation.id))?.pendingSwitch).toBeNull()

  // Switch real→virtual: the departed device is granted; the announcement points at `host shell --id`.
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: null }, 'PATCH')).status).toBe(200)
  record = await control.getConversation(conversation.id)
  expect(record?.workspaceId).toBeNull()
  expect(record?.pendingSwitch?.from).toEqual({ kind: 'workspace', workspaceId: workspace.id, deviceId: device.id, path: runnerDir })
  expect((await control.listHostGrants(conversation.id)).map((grant) => grant.deviceId)).toEqual([device.id])
  const listed = (await (await api(backend, `/api/conversations/${conversation.id}/grants`)).json()) as { grants: Array<{ deviceId: string }> }
  expect(listed.grants.map((grant) => grant.deviceId)).toEqual([device.id])

  await client.send([{ type: 'text', text: 'read from the old place' }])
  const reached = lastExited(shellEvents)
  expect(reached?.stdout.delta).toContain(`${device.id}  m6-device  online  ${runnerDir}  (granted)`)
  expect(reached?.stdout.delta).toContain('secret')
  expect(announcements(client)).toHaveLength(2)
  expect(announcements(client)[1]).toContain(`demi host shell --id ${device.id}`)
  expect(announcements(client)[1]).toContain(`tar c -C ${runnerDir} .`)

  // The grant API: a user device grants idempotently, a managed host is never grantable, revoke closes the door.
  expect((await json(backend, `/api/conversations/${conversation.id}/grants`, { deviceId: device.id })).status).toBe(201)
  expect(await control.listHostGrants(conversation.id)).toHaveLength(1)
  const managed = await control.createDevice({ userId: 'local', name: 'vm', platform: 'test', tokenHash: 'm', kind: 'managed', ownerConversationId: conversation.id })
  expect((await json(backend, `/api/conversations/${conversation.id}/grants`, { deviceId: managed.id })).status).toBe(404)
  const devices = (await (await api(backend, '/api/devices')).json()) as { devices: Array<{ id: string }> }
  expect(devices.devices.map((entry) => entry.id)).toEqual([device.id])
  expect((await api(backend, `/api/conversations/${conversation.id}/grants/${device.id}`, { method: 'DELETE' })).status).toBe(204)
  expect(await control.listHostGrants(conversation.id)).toEqual([])
  await client.send([{ type: 'text', text: 'try again' }])
  const refused = lastExited(shellEvents)
  expect(refused?.stderr.delta).toContain(`host ${device.id} is not reachable`)
  expect(refused?.stdout.delta).toContain('refused')

  // Offline degradation: with the runner gone, the session stays readable and
  // chattable — switching (a control-plane write) and text-only turns both work.
  await runner.stop()
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: workspace.id }, 'PATCH')).status).toBe(200)
  await client.send([{ type: 'text', text: 'still there?' }])
  expect(client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'turn 5')).toBe(true)
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: null }, 'PATCH')).status).toBe(200)

  // Workspace deletion: refused while bound, allowed when free.
  await control.setConversationWorkspace(conversation.id, workspace.id)
  expect((await api(backend, `/api/workspaces/${workspace.id}`, { method: 'DELETE' })).status).toBe(409)
  await control.setConversationWorkspace(conversation.id, null)
  expect((await api(backend, `/api/workspaces/${workspace.id}`, { method: 'DELETE' })).status).toBe(204)

  controlDb.close()
  await client.close()
  await runner.stop()
  await backend.close()
}, 30_000)

test('real→real switch: files stay, same-device note, the device granted once', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-runner-'))
  const scripts: string[] = []
  const backend = await createBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: ({ connectionId, label }) => defineProvider({ id: connectionId, displayName: label, createRuntime: scriptedStub(scripts) }),
    },
  })
  const selection = selectionFor(await stubConnection(backend))

  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'm6-rr-device' })
  await waitFor(() => runner.codes.length > 0, undefined, { timeoutMs: 5_000 })
  const { device } = (await (await json(backend, '/api/devices/claim', { code: runner.codes[0] })).json()) as { device: { id: string } }
  const dirA = join(runnerDir, 'a')
  const dirB = join(runnerDir, 'b')
  expect((await json(backend, `/api/devices/${device.id}/fs`, { path: dirA })).status).toBe(201)
  await json(backend, `/api/devices/${device.id}/fs`, { path: dirB })
  const workspaceFor = async (path: string, name: string) => {
    const response = await json(backend, '/api/workspaces', { deviceId: device.id, path, name })
    return ((await response.json()) as { workspace: { id: string } }).workspace
  }
  const workspaceA = await workspaceFor(dirA, 'A')
  const workspaceB = await workspaceFor(dirB, 'B')

  scripts.push(
    // Turn 1 (workspace A): leave a file behind.
    'printf alpha > a.txt',
    // Turn 2 (after A→B on the same device): the old directory is directly reachable, and so is the device as a host.
    `cat ${join(dirA, 'a.txt')} && demi host shell --id ${device.id} "cat ${join(dirA, 'a.txt')}"`,
  )

  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const patch = (body: unknown) => json(backend, `/api/conversations/${conversation.id}`, body, 'PATCH')
  expect((await patch({ workspaceId: workspaceA.id })).status).toBe(200)

  const { client, shellEvents } = await openClient(backend, conversation.id, selection)
  await client.send([{ type: 'text', text: 'write in A' }])
  expect(readFileSync(join(dirA, 'a.txt'), 'utf8')).toBe('alpha')

  // A→B: files stay in A; the context block carries the same-device note.
  expect((await patch({ workspaceId: workspaceB.id })).status).toBe(200)
  await client.send([{ type: 'text', text: 'now in B' }])
  expect(readFileSync(join(dirA, 'a.txt'), 'utf8')).toBe('alpha')
  expect(existsSync(join(dirB, 'a.txt'))).toBe(false)
  expect(lastExited(shellEvents)?.stdout.delta ?? '').toBe('alphaalpha')
  const note = announcements(client).at(-1) ?? ''
  expect(note).toContain(dirA)
  expect(note).toContain('same device')

  // Every switch away from the device grants it; the grant set holds it once.
  expect((await patch({ workspaceId: null })).status).toBe(200)
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control: ControlService = new LocalControlService(controlDb)
  const record = await control.getConversation(conversation.id)
  expect(record?.pendingSwitch?.from).toEqual({ kind: 'workspace', workspaceId: workspaceB.id, deviceId: device.id, path: dirB })
  expect((await control.listHostGrants(conversation.id)).map((grant) => grant.deviceId)).toEqual([device.id])
  controlDb.close()

  await client.close()
  await runner.stop()
  await backend.close()
}, 30_000)

test('a running turn refuses the switch; concurrent switches have one winner; a machine of its own has no hostless entrance', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-race-'))
  let releaseTurn: () => void = () => {}
  const gate = new Promise<void>((resolve) => {
    releaseTurn = resolve
  })
  const slowProvider: AgentProvider = {
    run: async function* (): AsyncIterable<ProviderEvent> {
      yield events.text('thinking…')
      await gate
      yield events.response()
    },
    clone(): AgentProvider {
      return this
    },
  }
  const backend = await createBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: ({ connectionId, label }) =>
        defineProvider({ id: connectionId, displayName: label, createRuntime: () => slowProvider }),
    },
  })
  const selection = selectionFor(await stubConnection(backend))
  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const { client } = await openClient(backend, conversation.id, selection)

  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control: ControlService = new LocalControlService(controlDb)
  const device = await control.createDevice({ userId: 'local', name: 'd', platform: 'test', tokenHash: 'x' })
  const workspace = await control.createWorkspace({ userId: 'local', deviceId: device.id, path: '/tmp', name: 'w' })

  // Mid-turn: the PATCH is refused with 409 while the provider is streaming.
  const sendPromise = client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text'), undefined, { timeoutMs: 5_000 })
  const refused = await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: workspace.id }, 'PATCH')
  expect(refused.status).toBe(409)
  expect(((await refused.json()) as { code: string }).code).toBe('turn_in_flight')
  releaseTurn()
  await sendPromise

  // At the boundary the same switch lands.
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: workspace.id }, 'PATCH')).status).toBe(200)

  // Concurrency: the compare-and-set gives exactly one winner from the same base.
  const fresh = await control.createConversation('local')
  const hostless = { workspaceId: null, hostDeviceId: null }
  const toWorkspace = { kind: 'workspace' as const, workspaceId: workspace.id, deviceId: device.id, path: '/tmp' }
  const results = await Promise.all([
    control.switchConversationTarget(fresh.id, hostless, { workspaceId: workspace.id, hostDeviceId: null }, { from: { kind: 'hostless' }, to: toWorkspace }, null),
    control.switchConversationTarget(fresh.id, hostless, hostless, { from: { kind: 'hostless' }, to: { kind: 'hostless' } }, null),
  ])
  expect(results.filter(Boolean)).toHaveLength(1)

  // A session-bound managed host: to a workspace is a switch that grants it; to hostless is refused.
  const managed = await control.createDevice({ userId: 'local', name: 'vm', platform: 'test', tokenHash: 'm', kind: 'managed', ownerConversationId: fresh.id })
  const bound = await control.createConversation('local')
  expect(
    await control.switchConversationTarget(bound.id, hostless, { workspaceId: null, hostDeviceId: managed.id }, { from: { kind: 'hostless' }, to: { kind: 'host', deviceId: managed.id } }, null),
  ).toBe(true)
  const noEntrance = await json(backend, `/api/conversations/${bound.id}`, { workspaceId: null }, 'PATCH')
  expect(noEntrance.status).toBe(409)
  expect(((await noEntrance.json()) as { code: string }).code).toBe('no_hostless_entrance')
  expect((await json(backend, `/api/conversations/${bound.id}`, { workspaceId: workspace.id }, 'PATCH')).status).toBe(200)
  const moved = await control.getConversation(bound.id)
  expect(moved?.hostDeviceId).toBeNull()
  expect(moved?.workspaceId).toBe(workspace.id)
  expect((await control.listHostGrants(bound.id)).map((grant) => grant.deviceId)).toEqual([managed.id])

  // Unknown workspace and unknown device surface as 404s on their routes.
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: 'nope' }, 'PATCH')).status).toBe(404)
  expect((await json(backend, '/api/workspaces', { deviceId: 'nope', path: '/tmp', name: 'w2' })).status).toBe(404)

  controlDb.close()
  await client.close()
  await backend.close()
}, 20_000)
