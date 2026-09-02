import { readFileSync, writeFileSync } from 'node:fs'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport, type ClientSessionEvent } from '@demicodes/agent'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import type { RunnerProtocolMessage } from '@demicodes/runner-protocol'
import { startTinyjsRunner } from '@demicodes/runner/testing'
import { waitFor } from '@demicodes/utils'
import { createBackend, type Backend } from '../index'

// M9 step 4: `demi host shell --id` between two devices. The script runs as
// a job on the named host; its stdout comes back as a brokered HTTP
// transfer the calling device GETs itself, so the working tree never
// crosses either runner socket — the wire audit below is the proof. A
// hostless caller takes the bytes in-process.

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
  const status = shellEvents.filter((event) => event.status.status === 'exited').at(-1)?.status
  return status?.status === 'exited' ? status : undefined
}

async function pairDevice(backend: Backend, name: string) {
  const home = await mkdtemp(join(tmpdir(), `demi-hs-${name}-`))
  const stateDir = await mkdtemp(join(tmpdir(), `demi-hs-${name}-state-`))
  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home, name })
  await waitFor(() => runner.codes.length > 0, () => runner.log.join('\n'), { timeoutMs: 10_000 })
  const claimed = await json(backend, '/api/devices/claim', { code: runner.codes[0] })
  const { device } = (await claimed.json()) as { device: { id: string } }
  await waitFor(() => runner.statuses.includes('online'))
  const created = await json(backend, '/api/workspaces', { deviceId: device.id, path: home, name: `${name} workspace` })
  const { workspace } = (await created.json()) as { workspace: { id: string } }
  return { home, runner, deviceId: device.id, workspaceId: workspace.id }
}

test('host shell --id: the job runs on the named host, its stdout arrives as a transfer, nothing bulk crosses the sockets', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-hs-data-'))
  /** Every message per device and direction: the wire audit. */
  const frames: Array<{ deviceId: string; direction: 'in' | 'out'; message: RunnerProtocolMessage }> = []
  const scripts: string[] = []
  const backend = await createBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0, trace: (deviceId, direction, message) => void frames.push({ deviceId, direction, message }) },
    providerTypes: {
      stub: ({ connectionId, label }) =>
        defineProvider({
          id: connectionId,
          displayName: label,
          createRuntime: () =>
            new StubProvider([
              [events.toolCall('t1', 'shell_exec', { script: scripts[0]!, timeoutMs: 30_000 })],
              [events.text('one'), events.response()],
              [events.toolCall('t2', 'shell_exec', { script: scripts[1]!, timeoutMs: 10_000 })],
              [events.text('two'), events.response()],
              [events.toolCall('t3', 'shell_exec', { script: scripts[2]!, timeoutMs: 30_000 })],
              [events.text('three'), events.response()],
            ]),
        }),
    },
  })
  const connection = (await (await json(backend, '/api/connections', { type: 'stub', label: 'Stub', apiKey: 'k' })).json()) as { connection: { id: string } }
  const selection = selectionFor(connection.connection.id)

  const a = await pairDevice(backend, 'alpha')
  const b = await pairDevice(backend, 'beta')
  // Well past the 32 KB view: only a transfer can carry it whole.
  const payload = Buffer.alloc(300 * 1024)
  for (let i = 0; i < payload.length; i += 1) payload[i] = (i * 31) & 0xff
  writeFileSync(join(a.home, 'notes.bin'), payload)

  // The conversation starts on alpha and switches to beta: alpha is its previous host, hence reachable.
  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: a.workspaceId }, 'PATCH')).status).toBe(200)
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: b.workspaceId }, 'PATCH')).status).toBe(200)

  scripts.push(
    `demi host list && demi host shell --id ${a.deviceId} "tar c -C ${a.home} notes.bin" | tar x && cmp notes.bin ${join(a.home, 'notes.bin')} && echo copied`,
    `demi host shell --id nope "echo hi"; echo exit=$?`,
    `demi host shell --id ${b.deviceId} "head -c 5 notes.bin | od -An -tx1"`,
  )
  const { client, shellEvents } = await openClient(backend, conversation.id, selection)

  const before = frames.length
  await client.send([{ type: 'text', text: 'copy the notes over' }])
  const copied = lastExited(shellEvents)
  expect(copied?.exitCode).toBe(0)
  expect(copied?.stdout.delta).toContain(`${a.deviceId}  alpha  online  ${a.home}  (previous)`)
  expect(copied?.stdout.delta).toContain(`${b.deviceId}  beta  online  ${b.home}  (current)`)
  expect(copied?.stdout.delta).toContain('copied')
  expect(readFileSync(join(b.home, 'notes.bin')).equals(payload)).toBe(true)
  // The audit: alpha ran a job (its view frames are the 32 KB head) and sent
  // the transfer; beta was told to fetch it, and the only `rpc_output` bytes
  // it received are `demi host list`'s lines — the archive went over HTTP.
  const turn = frames.slice(before)
  const of = (deviceId: string, direction: 'in' | 'out') => turn.filter((f) => f.deviceId === deviceId && f.direction === direction).map((f) => f.message)
  const types = (deviceId: string, direction: 'in' | 'out') => new Set(of(deviceId, direction).map((m) => m.type))
  expect(types(a.deviceId, 'out')).toEqual(new Set(['job_start', 'job_stdin_end', 'transfer_send']))
  expect(types(a.deviceId, 'in')).toEqual(new Set(['job_output', 'job_exit', 'transfer_done']))
  expect(types(b.deviceId, 'out').has('rpc_transfer')).toBe(true)
  const relayedBytes = of(b.deviceId, 'out').reduce((total, m) => total + (m.type === 'rpc_output' ? m.bytes.byteLength : 0), 0)
  expect(relayedBytes).toBeLessThan(1024)
  expect(of(a.deviceId, 'in').reduce((total, m) => total + (m.type === 'job_output' ? m.bytes.byteLength : 0), 0)).toBeLessThanOrEqual(32 * 1024)

  await client.send([{ type: 'text', text: 'try a stranger' }])
  const refused = lastExited(shellEvents)
  expect(refused?.stderr.delta).toContain('host nope is not reachable')
  expect(refused?.stdout.delta).toContain('exit=1')

  // Hostless caller: beta becomes the previous host, and the bytes land in this process's tinybash pipeline.
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: null }, 'PATCH')).status).toBe(200)
  await client.send([{ type: 'text', text: 'peek from nowhere' }])
  const peeked = lastExited(shellEvents)
  expect(peeked?.exitCode).toBe(0)
  expect(peeked?.stdout.delta.replace(/\s+/g, ' ').trim()).toBe('00 1f 3e 5d 7c')

  await a.runner.stop()
  await b.runner.stop()
  await backend.close()
}, 120_000)
