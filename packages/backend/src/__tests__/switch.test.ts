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

// M6: target switching — turn-boundary switch over the conversations PATCH,
// prev slot + context block, the `demi host` command frame, and the
// `prev shell` reaching a workspace prev over its runner (a hostless prev has
// no shell: the switch itself places its files, M11).

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

async function stubConnection(backend: Backend): Promise<string> {
  const response = await api(backend, '/api/connections', {
    method: 'POST',
    body: JSON.stringify({ type: 'stub', label: 'Stub', apiKey: 'test-key' }),
    headers: { 'content-type': 'application/json' },
  })
  const { connection } = (await response.json()) as { connection: { id: string } }
  return connection.id
}

test('M6 acceptance: virtual→real switch with context block and migration pipe, then real→virtual and release', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m6-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m6-runner-'))
  const stubRuntime = () =>
    new StubProvider([
      // Turn 1 (virtual): create a file that migration must carry over.
      [events.toolCall('t1', 'shell_exec', { script: 'echo -n secret > notes.txt && demi host current', timeoutMs: 10_000 })],
      [events.text('one'), events.response()],
      // Turn 2 (after switch to the device workspace): the hostless prev has no
      // shell to reach (its files are placed by the switch itself, M11); the
      // workspace is live, and a file written here is what the later prev reaches.
      [events.toolCall('t2', 'shell_exec', { script: 'demi host prev shell -- cat notes.txt; echo exit=$?; printf secret > notes.txt && demi host current', timeoutMs: 20_000 })],
      [events.text('two'), events.response()],
      // Turn 3: release, then the pipe is closed.
      [events.toolCall('t3', 'shell_exec', { script: 'demi host prev release; demi host prev shell -- ls; echo exit=$?', timeoutMs: 10_000 })],
      [events.text('three'), events.response()],
      // Turn 4 (after switch back to virtual): the old workspace is the prev, reachable by real spawn.
      [events.toolCall('t4', 'shell_exec', { script: 'demi host prev shell -- cat notes.txt', timeoutMs: 20_000 })],
      [events.text('four'), events.response()],
      // Turn 5: chat-only while the bound workspace's runner is offline.
      [events.text('offline chat works'), events.response()],
    ])
  const backend = await createBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: ({ connectionId, label }) => defineProvider({ id: connectionId, displayName: label, createRuntime: stubRuntime }),
    },
  })
  const selection = selectionFor(await stubConnection(backend))

  // Pair a device and create the workspace over the M6 HTTP surface.
  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'm6-device' })
  await waitFor(() => runner.codes.length > 0, undefined, { timeoutMs: 5_000 })
  const claimed = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: runner.codes[0] }),
    headers: { 'content-type': 'application/json' },
  })
  const { device } = (await claimed.json()) as { device: { id: string } }
  const workspaceResponse = await api(backend, '/api/workspaces', {
    method: 'POST',
    body: JSON.stringify({ deviceId: device.id, path: runnerDir, name: 'm6 workspace' }),
    headers: { 'content-type': 'application/json' },
  })
  expect(workspaceResponse.status).toBe(201)
  const { workspace } = (await workspaceResponse.json()) as { workspace: { id: string } }

  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const { client, shellEvents } = await openClient(backend, conversation.id, selection)

  // Turn 1 on virtual: file exists only in the virtual fs; status names virtual.
  await client.send([{ type: 'text', text: 'write a note' }])
  expect(lastExited(shellEvents)?.stdout.delta).toContain('host: virtual')
  expect(existsSync(join(runnerDir, 'notes.txt'))).toBe(false)

  // Switch virtual→real over PATCH; prev slot filled, not yet announced.
  const patched = await api(backend, `/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ workspaceId: workspace.id }),
    headers: { 'content-type': 'application/json' },
  })
  expect(patched.status).toBe(200)
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control: ControlService = new LocalControlService(controlDb)
  let record = await control.getConversation(conversation.id)
  expect(record?.workspaceId).toBe(workspace.id)
  expect(record?.prevTarget).toEqual({ target: { kind: 'virtual' }, announced: false })

  // Turn 2: the context block is injected; a hostless prev refuses `prev shell`.
  await client.send([{ type: 'text', text: 'bring my files' }])
  const migrated = lastExited(shellEvents)
  expect(migrated?.stderr.delta).toContain('previous target was hostless')
  expect(migrated?.stdout.delta).toContain('exit=1')
  expect(migrated?.stdout.delta).toContain('host: workspace "m6 workspace"')
  expect(readFileSync(join(runnerDir, 'notes.txt'), 'utf8')).toBe('secret')
  const announced = client
    .transcript()
    .blocks.some((block) => block.type === 'user' && block.preamble?.includes('[Execution target switched]'))
  expect(announced).toBe(true)
  record = await control.getConversation(conversation.id)
  expect(record?.prevTarget?.announced).toBe(true)

  // Turn 3: release closes the pipe.
  await client.send([{ type: 'text', text: 'done migrating' }])
  const released = lastExited(shellEvents)
  expect(released?.stdout.delta).toContain('exit=1')
  expect((await control.getConversation(conversation.id))?.prevTarget).toBeNull()

  // Switch real→virtual: the workspace becomes the prev, reachable by real spawn.
  const back = await api(backend, `/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ workspaceId: null }),
    headers: { 'content-type': 'application/json' },
  })
  expect(back.status).toBe(200)
  record = await control.getConversation(conversation.id)
  expect(record?.workspaceId).toBeNull()
  expect(record?.prevTarget?.target).toEqual({ kind: 'workspace', deviceId: device.id, path: runnerDir })

  await client.send([{ type: 'text', text: 'read from the old place' }])
  expect(lastExited(shellEvents)?.stdout.delta).toContain('secret')

  // Offline degradation: with the runner gone, the session stays readable and
  // chattable — switching (a control-plane write) and text-only turns both work.
  await runner.stop()
  const offlineSwitch = await api(backend, `/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ workspaceId: workspace.id }),
    headers: { 'content-type': 'application/json' },
  })
  expect(offlineSwitch.status).toBe(200)
  await client.send([{ type: 'text', text: 'still there?' }])
  expect(client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'offline chat works')).toBe(true)
  const backToVirtual = await api(backend, `/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ workspaceId: null }),
    headers: { 'content-type': 'application/json' },
  })
  expect(backToVirtual.status).toBe(200)

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

test('real→real switch: files stay, same-device note, prev slot single-occupancy', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-runner-'))
  const stubRuntime = () =>
    new StubProvider([
      // Turn 1 (workspace A): leave a file behind.
      [events.toolCall('t1', 'shell_exec', { script: 'printf alpha > a.txt', timeoutMs: 10_000 })],
      [events.text('one'), events.response()],
      // Turn 2 (after A→B on the same device): the old directory is directly reachable, and the pipe works too.
      [events.toolCall('t2', 'shell_exec', { script: `cat ${join(runnerDir, 'a', 'a.txt')} && demi host prev shell -- cat a.txt`, timeoutMs: 20_000 })],
      [events.text('two'), events.response()],
    ])
  const backend = await createBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: ({ connectionId, label }) => defineProvider({ id: connectionId, displayName: label, createRuntime: stubRuntime }),
    },
  })
  const selection = selectionFor(await stubConnection(backend))

  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'm6-rr-device' })
  await waitFor(() => runner.codes.length > 0, undefined, { timeoutMs: 5_000 })
  const claimed = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: runner.codes[0] }),
    headers: { 'content-type': 'application/json' },
  })
  const { device } = (await claimed.json()) as { device: { id: string } }
  const dirA = join(runnerDir, 'a')
  const dirB = join(runnerDir, 'b')
  const fsA = await api(backend, `/api/devices/${device.id}/fs`, {
    method: 'POST',
    body: JSON.stringify({ path: dirA }),
    headers: { 'content-type': 'application/json' },
  })
  expect(fsA.status).toBe(201)
  await api(backend, `/api/devices/${device.id}/fs`, {
    method: 'POST',
    body: JSON.stringify({ path: dirB }),
    headers: { 'content-type': 'application/json' },
  })
  const workspaceFor = async (path: string, name: string) => {
    const response = await api(backend, '/api/workspaces', {
      method: 'POST',
      body: JSON.stringify({ deviceId: device.id, path, name }),
      headers: { 'content-type': 'application/json' },
    })
    return ((await response.json()) as { workspace: { id: string } }).workspace
  }
  const workspaceA = await workspaceFor(dirA, 'A')
  const workspaceB = await workspaceFor(dirB, 'B')

  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const patch = (body: unknown) =>
    api(backend, `/api/conversations/${conversation.id}`, {
      method: 'PATCH',
      body: JSON.stringify(body),
      headers: { 'content-type': 'application/json' },
    })
  expect((await patch({ workspaceId: workspaceA.id })).status).toBe(200)

  const { client, shellEvents } = await openClient(backend, conversation.id, selection)
  await client.send([{ type: 'text', text: 'write in A' }])
  expect(readFileSync(join(dirA, 'a.txt'), 'utf8')).toBe('alpha')

  // A→B: files stay in A; the context block carries the same-device note.
  expect((await patch({ workspaceId: workspaceB.id })).status).toBe(200)
  await client.send([{ type: 'text', text: 'now in B' }])
  expect(readFileSync(join(dirA, 'a.txt'), 'utf8')).toBe('alpha')
  expect(existsSync(join(dirB, 'a.txt'))).toBe(false)
  const output = lastExited(shellEvents)?.stdout.delta ?? ''
  expect(output).toBe('alphaalpha')
  const sameDeviceNote = client
    .transcript()
    .blocks.some(
      (block) =>
        block.type === 'user' &&
        block.preamble?.includes('[Execution target switched]') &&
        block.preamble.includes(dirA) &&
        block.preamble.includes('same device'),
    )
  expect(sameDeviceNote).toBe(true)

  // Single occupancy: switching again without release replaces the prev slot.
  expect((await patch({ workspaceId: null })).status).toBe(200)
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control: ControlService = new LocalControlService(controlDb)
  const record = await control.getConversation(conversation.id)
  expect(record?.prevTarget?.target).toEqual({ kind: 'workspace', deviceId: device.id, path: dirB })
  controlDb.close()

  await client.close()
  await runner.stop()
  await backend.close()
}, 30_000)

test('a running turn refuses the switch; concurrent switches have one winner', async () => {
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
  const refused = await api(backend, `/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ workspaceId: workspace.id }),
    headers: { 'content-type': 'application/json' },
  })
  expect(refused.status).toBe(409)
  expect(((await refused.json()) as { code: string }).code).toBe('turn_in_flight')
  releaseTurn()
  await sendPromise

  // At the boundary the same switch lands.
  const accepted = await api(backend, `/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ workspaceId: workspace.id }),
    headers: { 'content-type': 'application/json' },
  })
  expect(accepted.status).toBe(200)

  // Concurrency: the compare-and-set gives exactly one winner from the same base.
  const fresh = await control.createConversation('local')
  const results = await Promise.all([
    control.switchConversationWorkspace(fresh.id, null, workspace.id, { target: { kind: 'virtual' }, announced: false }),
    control.switchConversationWorkspace(fresh.id, null, null, { target: { kind: 'virtual' }, announced: false }),
  ])
  expect(results.filter(Boolean)).toHaveLength(1)

  // Unknown workspace and unknown device surface as 404s on their routes.
  const badWorkspace = await api(backend, `/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ workspaceId: 'nope' }),
    headers: { 'content-type': 'application/json' },
  })
  expect(badWorkspace.status).toBe(404)
  const badDevice = await api(backend, '/api/workspaces', {
    method: 'POST',
    body: JSON.stringify({ deviceId: 'nope', path: '/tmp', name: 'w2' }),
    headers: { 'content-type': 'application/json' },
  })
  expect(badDevice.status).toBe(404)

  controlDb.close()
  await client.close()
  await backend.close()
}, 20_000)
