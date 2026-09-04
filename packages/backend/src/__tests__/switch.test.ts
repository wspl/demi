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
import { openBackend, type TestBackend } from './session'

// M6 acceptance, on the attached-hosts model: target switching at turn
// boundaries over the conversations PATCH, the pending switch and its
// context block, the departed device attached under its name and reached
// with `demi host shell --host`, the hosts API (attach any owned device,
// rename with names unique per conversation, detach closes the door), and
// offline degradation.

async function api(backend: TestBackend, path: string, init?: RequestInit): Promise<Response> {
  return backend.session.fetch(path, init)
}

async function json(backend: TestBackend, path: string, body: unknown, method = 'POST'): Promise<Response> {
  return api(backend, path, { method, body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })
}

function selectionFor(providerId: string) {
  const model: ModelSelection = {
    providerId: providerId,
    model: { id: 'm', name: 'M', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
    thinking: null,
  }
  return { providerId: providerId, model }
}

async function openClient(backend: TestBackend, conversationId: string, selection: ReturnType<typeof selectionFor>) {
  const socket = backend.session.socket(`/api/conversations/${conversationId}/stream`)
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

async function stubProviderId(backend: TestBackend): Promise<string> {
  const response = await json(backend, '/api/providers', { providerType: 'stub', label: 'Stub', apiKey: 'test-key' })
  const { provider } = (await response.json()) as { provider: { id: string } }
  return provider.id
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

test('M6 acceptance: virtual→real switch with context block, real→virtual attaches the device, hosts API, offline chat', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m6-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m6-runner-'))
  const scripts: string[] = []
  const backend = await openBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: { credential: 'api_key', create: ({ providerId, label }) => defineProvider({ id: providerId, displayName: label, createRuntime: scriptedStub(scripts) }) },
    },
  })
  const selection = selectionFor(await stubProviderId(backend))

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
    // Turn 2 (after the switch to the workspace): only the main host is reachable; a file written here is what the attachment reaches later.
    'demi host list; printf secret > notes.txt && demi host current',
    // Turn 3 (after the switch back to virtual): the workspace's device is attached under its name, its shell starts in the directory it was left at.
    'demi host list && demi host shell --host m6-device "cat notes.txt"',
    // Turn 4 (after the detach): the door is closed.
    `demi host shell --host ${device.id} "cat notes.txt" || echo refused`,
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

  // Switch virtual→real over PATCH: the switch is pending, nothing attached (hostless has no device).
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: workspace.id }, 'PATCH')).status).toBe(200)
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control: ControlService = new LocalControlService(controlDb)
  let record = await control.getConversation(conversation.id)
  expect(record?.workspaceId).toBe(workspace.id)
  expect(record?.pendingSwitch).toEqual({
    from: { kind: 'hostless' },
    to: { kind: 'workspace', workspaceId: workspace.id, deviceId: device.id, path: runnerDir },
  })
  expect(await control.listAttachedHosts(conversation.id)).toEqual([])

  // Turn 2: the context block is injected once; the workspace is live.
  await client.send([{ type: 'text', text: 'bring my files' }])
  const onWorkspace = lastExited(shellEvents)
  expect(onWorkspace?.stdout.delta).toContain(`m6-device  ${device.id}  online  ${runnerDir}  (main)`)
  expect(onWorkspace?.stdout.delta).toContain('host: workspace "m6 workspace"')
  expect(readFileSync(join(runnerDir, 'notes.txt'), 'utf8')).toBe('secret')
  expect(announcements(client)).toHaveLength(1)
  expect(announcements(client)[0]).toContain('Previous target: the virtual environment')
  expect(announcements(client)[0]).not.toContain('demi host shell')
  expect((await control.getConversation(conversation.id))?.pendingSwitch).toBeNull()

  // Switch real→virtual: the departed device is attached under its name at the directory it was left at; the announcement points at `host shell --host`.
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: null }, 'PATCH')).status).toBe(200)
  record = await control.getConversation(conversation.id)
  expect(record?.workspaceId).toBeNull()
  expect(record?.pendingSwitch?.from).toEqual({ kind: 'workspace', workspaceId: workspace.id, deviceId: device.id, path: runnerDir })
  expect((await control.listAttachedHosts(conversation.id)).map((host) => [host.deviceId, host.name, host.cwd])).toEqual([[device.id, 'm6-device', runnerDir]])
  const listed = (await (await api(backend, `/api/conversations/${conversation.id}/hosts`)).json()) as { hosts: Array<{ deviceId: string; name: string; online: boolean }> }
  expect(listed.hosts.map((host) => [host.deviceId, host.name, host.online])).toEqual([[device.id, 'm6-device', true]])

  await client.send([{ type: 'text', text: 'read from the old place' }])
  const reached = lastExited(shellEvents)
  expect(reached?.stdout.delta).toContain(`m6-device  ${device.id}  online  ${runnerDir}  (attached)`)
  expect(reached?.stdout.delta).toContain('secret')
  expect(announcements(client)).toHaveLength(2)
  expect(announcements(client)[1]).toContain('stays attached as "m6-device"')
  expect(announcements(client)[1]).toContain('demi host shell --host m6-device')
  expect(announcements(client)[1]).toContain(`tar c -C ${runnerDir} .`)
  expect(announcements(client)[1]).toContain(`Attached hosts: "m6-device" (online, shells start in ${runnerDir})`)

  // The hosts API: attaching is idempotent, any owned device attaches (the product surface decides what it offers), a rename must be unique, detach closes the door.
  expect((await json(backend, `/api/conversations/${conversation.id}/hosts`, { deviceId: device.id })).status).toBe(201)
  expect(await control.listAttachedHosts(conversation.id)).toHaveLength(1)
  const managed = await control.createDevice({ userId: backend.session.user.id, name: 'vm', platform: 'test', tokenHash: 'm', kind: 'managed', ownerConversationId: conversation.id })
  expect((await json(backend, `/api/conversations/${conversation.id}/hosts`, { deviceId: managed.id })).status).toBe(201)
  expect((await control.listAttachedHosts(conversation.id)).map((host) => host.name)).toEqual(['m6-device', 'vm'])
  expect((await json(backend, `/api/conversations/${conversation.id}/hosts/${managed.id}`, { name: 'm6-device' }, 'PATCH')).status).toBe(409)
  expect((await json(backend, `/api/conversations/${conversation.id}/hosts/${managed.id}`, { name: 'cloud' }, 'PATCH')).status).toBe(200)
  expect((await control.listAttachedHosts(conversation.id)).map((host) => host.name)).toEqual(['m6-device', 'cloud'])
  expect((await control.getConversation(conversation.id))?.hostsChanged).toBe(true)
  const devices = (await (await api(backend, '/api/devices')).json()) as { devices: Array<{ id: string }> }
  expect(devices.devices.map((entry) => entry.id)).toEqual([device.id])
  expect((await api(backend, `/api/conversations/${conversation.id}/hosts/${device.id}`, { method: 'DELETE' })).status).toBe(204)
  expect((await api(backend, `/api/conversations/${conversation.id}/hosts/${managed.id}`, { method: 'DELETE' })).status).toBe(204)
  expect(await control.listAttachedHosts(conversation.id)).toEqual([])
  await client.send([{ type: 'text', text: 'try again' }])
  // The change was announced once, as the set now stands.
  expect(client.transcript().blocks.filter((block) => block.type === 'user' && block.preamble?.includes('[Attached hosts changed]')).map((block) => (block.type === 'user' ? block.preamble : ''))).toEqual([
    expect.stringContaining('Attached hosts: none.'),
  ])
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

test('real→real switch: files stay, same-device note, the device attached once', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m6-rr-runner-'))
  const scripts: string[] = []
  const backend = await openBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: { credential: 'api_key', create: ({ providerId, label }) => defineProvider({ id: providerId, displayName: label, createRuntime: scriptedStub(scripts) }) },
    },
  })
  const selection = selectionFor(await stubProviderId(backend))

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
    // Turn 2 (after A→B on the same device): the old directory is directly reachable; the device is the main host, so `--host` names it too.
    `cat ${join(dirA, 'a.txt')} && demi host shell --host ${device.id} "cat ${join(dirA, 'a.txt')}"`,
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

  // A switch within the device attaches nothing (main is main); the switch away attaches it once, at the directory it was left at.
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control: ControlService = new LocalControlService(controlDb)
  expect(await control.listAttachedHosts(conversation.id)).toEqual([])
  expect((await patch({ workspaceId: null })).status).toBe(200)
  const record = await control.getConversation(conversation.id)
  expect(record?.pendingSwitch?.from).toEqual({ kind: 'workspace', workspaceId: workspaceB.id, deviceId: device.id, path: dirB })
  expect((await control.listAttachedHosts(conversation.id)).map((host) => [host.deviceId, host.cwd])).toEqual([[device.id, dirB]])
  // Switching back onto the attached device makes it main again: the row goes.
  expect((await patch({ workspaceId: workspaceA.id })).status).toBe(200)
  expect(await control.listAttachedHosts(conversation.id)).toEqual([])
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
  const backend = await openBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: { credential: 'api_key', create: ({ providerId, label }) =>
        defineProvider({ id: providerId, displayName: label, createRuntime: () => slowProvider }) },
    },
  })
  const selection = selectionFor(await stubProviderId(backend))
  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const { client } = await openClient(backend, conversation.id, selection)

  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control: ControlService = new LocalControlService(controlDb)
  const device = await control.createDevice({ userId: backend.session.user.id, name: 'd', platform: 'test', tokenHash: 'x' })
  const workspace = await control.createWorkspace({ userId: backend.session.user.id, deviceId: device.id, path: '/tmp', name: 'w' })

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
  const fresh = await control.createConversation(backend.session.user.id)
  const hostless = { workspaceId: null, hostDeviceId: null }
  const toWorkspace = { kind: 'workspace' as const, workspaceId: workspace.id, deviceId: device.id, path: '/tmp' }
  const noEnds = { departed: null, arrivingDeviceId: null }
  const results = await Promise.all([
    control.switchConversationTarget(fresh.id, hostless, { workspaceId: workspace.id, hostDeviceId: null }, { from: { kind: 'hostless' }, to: toWorkspace }, { departed: null, arrivingDeviceId: device.id }),
    control.switchConversationTarget(fresh.id, hostless, hostless, { from: { kind: 'hostless' }, to: { kind: 'hostless' } }, noEnds),
  ])
  expect(results.filter(Boolean)).toHaveLength(1)

  // Names are unique within a conversation: two devices with one hostname attach as `d` and `d-2`.
  const twin = await control.createDevice({ userId: backend.session.user.id, name: 'd', platform: 'test', tokenHash: 'y' })
  expect((await control.attachHost(fresh.id, twin.id, twin.name, null, true)).name).toBe('d')
  const third = await control.createDevice({ userId: backend.session.user.id, name: 'd', platform: 'test', tokenHash: 'z' })
  expect((await control.attachHost(fresh.id, third.id, third.name, '/srv', true)).name).toBe('d-2')
  expect((await control.attachHost(fresh.id, third.id, 'other', null, true)).name).toBe('d-2')
  expect(await control.renameAttachedHost(fresh.id, third.id, 'd')).toBe('name_taken')
  expect(await control.renameAttachedHost(fresh.id, third.id, 'ci')).toBe('renamed')
  expect(await control.renameAttachedHost(fresh.id, 'nope', 'ci')).toBe('not_attached')
  await control.setAttachedHostCwd(fresh.id, third.id, '/srv/app')
  expect((await control.listAttachedHosts(fresh.id)).map((host) => [host.name, host.cwd]).sort()).toEqual([['ci', '/srv/app'], ['d', null]])

  // A session-bound managed host: to a workspace is a switch that attaches it; to hostless is refused.
  const managed = await control.createDevice({ userId: backend.session.user.id, name: 'vm', platform: 'test', tokenHash: 'm', kind: 'managed', ownerConversationId: fresh.id })
  const bound = await control.createConversation(backend.session.user.id)
  expect(
    await control.switchConversationTarget(bound.id, hostless, { workspaceId: null, hostDeviceId: managed.id }, { from: { kind: 'hostless' }, to: { kind: 'host', deviceId: managed.id } }, { departed: null, arrivingDeviceId: managed.id }),
  ).toBe(true)
  const noEntrance = await json(backend, `/api/conversations/${bound.id}`, { workspaceId: null }, 'PATCH')
  expect(noEntrance.status).toBe(409)
  expect(((await noEntrance.json()) as { code: string }).code).toBe('no_hostless_entrance')
  expect((await json(backend, `/api/conversations/${bound.id}`, { workspaceId: workspace.id }, 'PATCH')).status).toBe(200)
  const moved = await control.getConversation(bound.id)
  expect(moved?.hostDeviceId).toBeNull()
  expect(moved?.workspaceId).toBe(workspace.id)
  expect((await control.listAttachedHosts(bound.id)).map((host) => [host.deviceId, host.name, host.cwd])).toEqual([[managed.id, 'vm', null]])

  // Unknown workspace and unknown device surface as 404s on their routes.
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: 'nope' }, 'PATCH')).status).toBe(404)
  expect((await json(backend, '/api/workspaces', { deviceId: 'nope', path: '/tmp', name: 'w2' })).status).toBe(404)

  controlDb.close()
  await client.close()
  await backend.close()
}, 20_000)
