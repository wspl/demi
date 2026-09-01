import { existsSync, readFileSync } from 'node:fs'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport, type ClientSessionEvent } from '@demicodes/agent'
import { LocalHost } from '@demicodes/host-local'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { encodeRunnerMessage } from '@demicodes/runner-protocol'
import { RunnerClient } from '@demicodes/runner'
import { delay, waitFor } from '@demicodes/utils'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { createBackend, type Backend } from '../index'

// M4: the pairing flow end-to-end — unclaimed runner prints a code, the web
// API claims it, the device token round-trips a restart, revoke refuses the
// reconnect, and the browse endpoints ride the same Host RPC.

const NO_PROVIDERS: never[] = []

async function api(backend: Backend, path: string, init?: RequestInit): Promise<Response> {
  return fetch(`${backend.url}${path}`, init)
}

function startRunner(backend: Backend, dirs: { stateDir: string; runnerDir: string }, capture: RunnerCapture) {
  const runner = new RunnerClient({
    backendUrl: backend.url,
    stateDir: dirs.stateDir,
    name: 'test-device',
    host: new LocalHost(dirs.runnerDir),
    reconnect: { initialDelayMs: 30, maxDelayMs: 100 },
    onStatus: (status, detail) => {
      capture.statuses.push(status)
      if (detail) capture.details.push(detail)
    },
    onClaimPending: (code) => {
      capture.codes.push(code)
    },
  })
  runner.start()
  return runner
}

interface RunnerCapture {
  codes: string[]
  statuses: string[]
  details: string[]
}

function capture(): RunnerCapture {
  return { codes: [], statuses: [], details: [] }
}

test('pairing: claim, reconnect with the device token, revoke refuses the reconnect', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m4-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m4-runner-'))
  const backend = await createBackend({ dataDir, providers: NO_PROVIDERS, port: 0, runner: { pingIntervalMs: 0 } })

  const first = capture()
  const runner = startRunner(backend, { stateDir, runnerDir }, first)
  await waitFor(() => first.codes.length > 0, undefined, { timeoutMs: 5_000 })
  const code = first.codes[0]
  expect(code).toMatch(/^[0-9A-Z]{4}(-[0-9A-Z]{1,4}){6}$|^[0-9A-Z-]{26,}$/)

  // A wrong code is refused; the right one claims — entered messy, normalized server-side.
  const wrong = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: 'AAAA-BBBB' }),
    headers: { 'content-type': 'application/json' },
  })
  expect(wrong.status).toBe(404)
  const claimed = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: ` ${code.toLowerCase()} ` }),
    headers: { 'content-type': 'application/json' },
  })
  expect(claimed.status).toBe(201)
  const { device } = (await claimed.json()) as { device: { id: string; name: string; online: boolean } }
  expect(device.name).toBe('test-device')
  await waitFor(() => first.statuses.includes('online'))

  // Claiming is single-use: the spent code is gone.
  const reuse = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code }),
    headers: { 'content-type': 'application/json' },
  })
  expect(reuse.status).toBe(404)

  const listed = (await (await api(backend, '/api/devices')).json()) as {
    devices: Array<{ id: string; online: boolean }>
  }
  expect(listed.devices).toEqual([expect.objectContaining({ id: device.id, online: true })])

  // Directory browse and create ride the runner's Host RPC.
  await writeFile(join(runnerDir, 'hello.txt'), 'hi')
  const browse = (await (
    await api(backend, `/api/devices/${device.id}/fs?path=${encodeURIComponent(runnerDir)}`)
  ).json()) as { entries: Array<{ name: string; isDirectory: boolean }> }
  expect(browse.entries).toContainEqual({ name: 'hello.txt', isDirectory: false })
  const makeDir = await api(backend, `/api/devices/${device.id}/fs`, {
    method: 'POST',
    body: JSON.stringify({ path: join(runnerDir, 'made/by/web') }),
    headers: { 'content-type': 'application/json' },
  })
  expect(makeDir.status).toBe(201)
  expect(existsSync(join(runnerDir, 'made/by/web'))).toBe(true)

  // Restart the runner: the persisted device token authenticates directly.
  await runner.stop()
  let offline = false
  for (let tries = 0; tries < 100 && !offline; tries += 1) {
    const rows = (await (await api(backend, '/api/devices')).json()) as { devices: Array<{ online: boolean }> }
    offline = rows.devices[0]?.online === false
    if (!offline) await delay(20)
  }
  expect(offline).toBe(true)
  const second = capture()
  const restarted = startRunner(backend, { stateDir, runnerDir }, second)
  await waitFor(() => second.statuses.includes('online'), undefined, { timeoutMs: 5_000 })
  expect(second.codes).toHaveLength(0)

  // Revoke: the row disappears and the live connection is refused for good.
  const revoked = await api(backend, `/api/devices/${device.id}`, { method: 'DELETE' })
  expect(revoked.status).toBe(204)
  await waitFor(() => second.statuses.includes('rejected'))
  expect(second.details.some((detail) => detail.includes('revoked'))).toBe(true)
  const after = (await (await api(backend, '/api/devices')).json()) as { devices: unknown[] }
  expect(after.devices).toHaveLength(0)

  await restarted.stop()
  await backend.close()
}, 30_000)

test('claim codes expire and rotate on the waiting socket; the stale code is dead', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-expiry-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m4-expiry-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m4-expiry-runner-'))
  const backend = await createBackend({
    dataDir,
    providers: NO_PROVIDERS,
    port: 0,
    runner: { pingIntervalMs: 0, claimTtlMs: 150 },
  })
  const seen = capture()
  const runner = startRunner(backend, { stateDir, runnerDir }, seen)
  await waitFor(() => seen.codes.length >= 2, undefined, { timeoutMs: 5_000 })
  expect(seen.codes[1]).not.toBe(seen.codes[0])

  const stale = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: seen.codes[0] }),
    headers: { 'content-type': 'application/json' },
  })
  expect(stale.status).toBe(404)
  const fresh = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: seen.codes[seen.codes.length - 1] }),
    headers: { 'content-type': 'application/json' },
  })
  expect(fresh.status).toBe(201)

  await runner.stop()
  await backend.close()
}, 15_000)

test('the claim endpoint is rate-limited per user', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-rate-'))
  const backend = await createBackend({
    dataDir,
    providers: NO_PROVIDERS,
    port: 0,
    runner: { pingIntervalMs: 0, claimAttemptsPerMinute: 2 },
  })
  const attempt = () =>
    api(backend, '/api/devices/claim', {
      method: 'POST',
      body: JSON.stringify({ code: 'NOPE-NOPE' }),
      headers: { 'content-type': 'application/json' },
    })
  expect((await attempt()).status).toBe(404)
  expect((await attempt()).status).toBe(404)
  expect((await attempt()).status).toBe(429)
  await backend.close()
})

test('a malformed runner frame closes the socket; a bad device token is rejected', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-reject-'))
  const backend = await createBackend({ dataDir, providers: NO_PROVIDERS, port: 0, runner: { pingIntervalMs: 0 } })
  const wsUrl = `${backend.url.replace('http', 'ws')}/api/runner`

  const garbage = new WebSocket(wsUrl)
  await new Promise<void>((resolve) => garbage.addEventListener('open', () => resolve(), { once: true }))
  const closed = new Promise<void>((resolve) => garbage.addEventListener('close', () => resolve(), { once: true }))
  garbage.send('{"type":"not-a-runner-frame"}')
  await closed

  const badToken = new WebSocket(wsUrl)
  await new Promise<void>((resolve) => badToken.addEventListener('open', () => resolve(), { once: true }))
  const reply = new Promise<string>((resolve) =>
    badToken.addEventListener('message', (event) => resolve(String(event.data)), { once: true }),
  )
  badToken.send(
    encodeRunnerMessage({
      type: 'hello',
      protocol: 1,
      deviceToken: 'not-a-real-token',
      runner: { name: 'x', platform: 'test', version: '0', identity: { uid: 0, gid: 0, hostname: 'x' } },
    }),
  )
  expect(JSON.parse(await reply)).toMatchObject({ type: 'hello_error', reason: 'unknown device' })
  badToken.close()
  await backend.close()
}, 15_000)

const model: ModelSelection = {
  providerId: 'stub',
  model: { id: 'm', name: 'M', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
}
const selection = { providerId: 'stub', model }

test('M4 acceptance: a session executes on the claimed device; disconnect is a tool error; reconnect resumes', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-accept-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m4-accept-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m4-accept-runner-'))
  const provider = defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () =>
      new StubProvider([
        // Turn 1: real spawns on the claimed device.
        [events.toolCall('t1', 'shell_exec', { script: 'printf hello | tee made.txt | cat', timeoutMs: 10_000 })],
        [events.text('turn one done'), events.response()],
        // Turn 2: killed mid-command by the runner going away.
        [events.toolCall('t2', 'shell_exec', { script: 'touch started.marker && sleep 30', timeoutMs: 60_000 })],
        [events.text('survived the drop'), events.response()],
        // Turn 3: after the runner comes back, the same session serves again.
        [events.toolCall('t3', 'shell_exec', { script: 'cat made.txt', timeoutMs: 10_000 })],
        [events.text('turn three done'), events.response()],
      ]),
  })
  const backend = await createBackend({ dataDir, providers: [provider], port: 0, runner: { pingIntervalMs: 0 } })

  // Pair a device, then point the conversation's workspace at it (the M6
  // workspace endpoints do this over HTTP; here the control plane is written
  // directly).
  const paired = capture()
  const runner = startRunner(backend, { stateDir, runnerDir }, paired)
  await waitFor(() => paired.codes.length > 0, undefined, { timeoutMs: 5_000 })
  const claimed = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: paired.codes[0] }),
    headers: { 'content-type': 'application/json' },
  })
  const { device } = (await claimed.json()) as { device: { id: string } }

  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control = new LocalControlService(controlDb)
  const workspace = await control.createWorkspace({
    userId: 'local',
    deviceId: device.id,
    path: runnerDir,
    name: 'test workspace',
  })
  await control.setConversationWorkspace(conversation.id, workspace.id)
  controlDb.close()

  const socket = new WebSocket(`${backend.url.replace('http', 'ws')}/api/conversations/${conversation.id}/stream`)
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

  // Turn 1: the command runs on the device — the file really exists there.
  await client.send([{ type: 'text', text: 'run on my machine' }])
  expect(shellEvents.filter((event) => event.status.status === 'exited').at(-1)?.status.stdout.delta).toBe('hello')
  expect(readFileSync(join(runnerDir, 'made.txt'), 'utf8')).toBe('hello')

  // Turn 2: stop the runner while `sleep 30` runs — an ordinary tool error,
  // the turn completes, the session survives.
  const sendPromise = client.send([{ type: 'text', text: 'now hang' }])
  await waitFor(() => existsSync(join(runnerDir, 'started.marker')), undefined, { timeoutMs: 10_000 })
  await runner.stop()
  await sendPromise
  const failed = shellEvents.at(-1)
  expect(failed?.status.status).not.toBe('running')

  // A fresh runner process with the persisted device token: reconnect resumes.
  const returned = capture()
  const revived = startRunner(backend, { stateDir, runnerDir }, returned)
  await waitFor(() => returned.statuses.includes('online'), undefined, { timeoutMs: 5_000 })
  const eventsBeforeTurn3 = shellEvents.length
  await client.send([{ type: 'text', text: 'read it back' }])
  const turn3 = shellEvents.slice(eventsBeforeTurn3).at(-1)
  expect(turn3?.status.status).toBe('exited')
  expect(turn3?.status.stdout.delta).toBe('hello')

  await client.close()
  await revived.stop()
  await backend.close()
}, 30_000)
