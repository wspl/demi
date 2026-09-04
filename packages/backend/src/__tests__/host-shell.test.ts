import { readFileSync, realpathSync, writeFileSync } from 'node:fs'
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
import { openBackend, type TestBackend } from './session'

// `demi host shell --host` between two devices (`runner.md` § Pipes). The
// script runs as a job on the named host with the caller's stdin and stdout
// attached to the job's as pipes: HTTP streams between the two runners,
// brokered by the backend, so a working tree never crosses either runner
// socket in either direction — the wire audit below is the proof. A
// hostless caller's ends are the backend's own streams.

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
  const status = shellEvents.filter((event) => event.status.status === 'exited').at(-1)?.status
  return status?.status === 'exited' ? status : undefined
}

async function pairDevice(backend: TestBackend, name: string) {
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

test('host shell --host: the job runs on the named host with the caller\'s pipes attached both ways, nothing bulk crosses the sockets; the directory carries', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-hs-data-'))
  /** Every message per device and direction: the wire audit. */
  const frames: Array<{ deviceId: string; direction: 'in' | 'out'; message: RunnerProtocolMessage }> = []
  const scripts: string[] = []
  const backend = await openBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0, trace: (deviceId, direction, message) => void frames.push({ deviceId, direction, message }) },
    providerTypes: {
      stub: {
        credential: 'api_key',
        create: ({ providerId, label }) =>
        defineProvider({
          id: providerId,
          displayName: label,
          createRuntime: () =>
            new StubProvider([
              [events.toolCall('t1', 'shell_exec', { script: scripts[0]!, timeoutMs: 30_000 })],
              [events.text('one'), events.response()],
              [events.toolCall('t2', 'shell_exec', { script: scripts[1]!, timeoutMs: 30_000 })],
              [events.text('two'), events.response()],
              [events.toolCall('t3', 'shell_exec', { script: scripts[2]!, timeoutMs: 10_000 })],
              [events.text('three'), events.response()],
              [events.toolCall('t4', 'shell_exec', { script: scripts[3]!, timeoutMs: 30_000 })],
              [events.text('four'), events.response()],
              [events.toolCall('t5', 'shell_exec', { script: scripts[4]!, timeoutMs: 30_000 })],
              [events.text('five'), events.response()],
            ]),
        }),
      },
    },
  })
  const provider = (await (await json(backend, '/api/providers', { providerType: 'stub', label: 'Stub', apiKey: 'k' })).json()) as { provider: { id: string } }
  const selection = selectionFor(provider.provider.id)

  const a = await pairDevice(backend, 'alpha')
  const b = await pairDevice(backend, 'beta')
  // Well past the 32 KB view: only a transfer can carry it whole.
  const payload = Buffer.alloc(300 * 1024)
  for (let i = 0; i < payload.length; i += 1) payload[i] = (i * 31) & 0xff
  writeFileSync(join(a.home, 'notes.bin'), payload)

  // The conversation starts on alpha and switches to beta: the switch attached alpha under its name, hence reachable.
  const created = await api(backend, '/api/conversations', { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: a.workspaceId }, 'PATCH')).status).toBe(200)
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: b.workspaceId }, 'PATCH')).status).toBe(200)

  scripts.push(
    `demi host list && demi host shell --host alpha "tar c -C ${a.home} notes.bin" | tar x && cmp notes.bin ${join(a.home, 'notes.bin')} && echo copied`,
    `head -c 250000 notes.bin > push.bin && tar c push.bin | demi host shell --host alpha "tar x -C ${a.home}" && cmp push.bin ${join(a.home, 'push.bin')} && echo pushed`,
    // Where a shell on the attached host ends is where the next one starts; `--host` takes the id as well.
    `demi host shell --host alpha "mkdir -p sub && cd sub && pwd" && demi host shell --host ${a.deviceId} "pwd" && demi host list`,
    `demi host shell --host nope "echo hi"; echo exit=$?`,
    `demi host shell --host beta "head -c 5 notes.bin | od -An -tx1"`,
  )
  const { client, shellEvents } = await openClient(backend, conversation.id, selection)

  const before = frames.length
  await client.send([{ type: 'text', text: 'copy the notes over' }])
  const copied = lastExited(shellEvents)
  expect(copied?.exitCode).toBe(0)
  expect(copied?.stdout.delta).toContain(`beta  ${b.deviceId}  online  ${b.home}  (main)`)
  expect(copied?.stdout.delta).toContain(`alpha  ${a.deviceId}  online  ${a.home}  (attached)`)
  expect(copied?.stdout.delta).toContain('copied')
  expect(readFileSync(join(b.home, 'notes.bin')).equals(payload)).toBe(true)
  // The audit: alpha ran a job with its stdout attached to a pipe (its view
  // frames are the 32 KB head); beta was named the pipe's sink in
  // `rpc_pipes`, and `rpc_output` carries only stderr — nothing of the
  // archive, which went over HTTP. `demi host list`'s lines went over
  // beta's stdout pipe too.
  const audit = (from: number) => {
    const turn = frames.slice(from)
    const of = (deviceId: string, direction: 'in' | 'out') => turn.filter((f) => f.deviceId === deviceId && f.direction === direction).map((f) => f.message)
    const types = (deviceId: string, direction: 'in' | 'out') => new Set(of(deviceId, direction).map((m) => m.type))
    return { of, types }
  }
  {
    const { of, types } = audit(before)
    expect(types(a.deviceId, 'out')).toEqual(new Set(['job_start']))
    expect(types(a.deviceId, 'in')).toEqual(new Set(['job_output', 'job_exit', 'pipe_done']))
    expect(types(b.deviceId, 'out').has('rpc_pipes')).toBe(true)
    expect(of(b.deviceId, 'out').some((m) => m.type === 'rpc_output')).toBe(false)
    expect(of(a.deviceId, 'in').reduce((total, m) => total + (m.type === 'job_output' ? m.bytes.byteLength : 0), 0)).toBeLessThanOrEqual(32 * 1024)
    expect(of(a.deviceId, 'in').filter((m) => m.type === 'pipe_done').every((m) => m.type === 'pipe_done' && m.ok)).toBe(true)
  }

  // The other direction: beta's pipe is alpha's job's stdin. The runner sockets carry the same control frames and nothing of the archive.
  const beforePush = frames.length
  await client.send([{ type: 'text', text: 'push one back' }])
  const pushed = lastExited(shellEvents)
  expect(pushed?.exitCode).toBe(0)
  expect(pushed?.stdout.delta).toContain('pushed')
  expect(readFileSync(join(a.home, 'push.bin')).equals(payload.subarray(0, 250_000))).toBe(true)
  // `tar x` prints nothing, so alpha sends no view frames; its stdin end reports once the body is in.
  await waitFor(() => frames.slice(beforePush).some((f) => f.deviceId === a.deviceId && f.message.type === 'pipe_done'))
  {
    const { of, types } = audit(beforePush)
    expect(types(a.deviceId, 'out')).toEqual(new Set(['job_start']))
    expect(types(a.deviceId, 'in')).toEqual(new Set(['job_exit', 'pipe_done']))
    const pipes = of(b.deviceId, 'out').find((m) => m.type === 'rpc_pipes')
    expect(pipes?.type === 'rpc_pipes' && pipes.stdin !== undefined).toBe(true)
    expect(of(b.deviceId, 'in').filter((m) => m.type === 'rpc_call').every((m) => m.type === 'rpc_call' && m.stdin)).toBe(true)
    const started = of(a.deviceId, 'out').find((m) => m.type === 'job_start')
    expect(started?.type === 'job_start' && started.stdin?.id === (pipes?.type === 'rpc_pipes' ? pipes.stdin?.id : undefined)).toBe(true)
  }

  await client.send([{ type: 'text', text: 'move around over there' }])
  const moved = lastExited(shellEvents)
  expect(moved?.exitCode).toBe(0)
  // `pwd` and the recorded directory are the resolved path (`/private/var` on macOS).
  const sub = join(realpathSync(a.home), 'sub')
  expect(moved?.stdout.delta).toContain(`${sub}\n${sub}\n`)
  expect(moved?.stdout.delta).toContain(`alpha  ${a.deviceId}  online  ${sub}  (attached)`)

  await client.send([{ type: 'text', text: 'try a stranger' }])
  const refused = lastExited(shellEvents)
  expect(refused?.stderr.delta).toContain('host nope is not reachable')
  expect(refused?.stdout.delta).toContain('exit=1')

  // Hostless caller: beta is attached by the switch, and the bytes land in this process's tinybash pipeline through the backend's own end.
  expect((await json(backend, `/api/conversations/${conversation.id}`, { workspaceId: null }, 'PATCH')).status).toBe(200)
  await client.send([{ type: 'text', text: 'peek from nowhere' }])
  const peeked = lastExited(shellEvents)
  expect(peeked?.exitCode).toBe(0)
  expect(peeked?.stdout.delta.replace(/\s+/g, ' ').trim()).toBe('00 1f 3e 5d 7c')

  await a.runner.stop()
  await b.runner.stop()
  await backend.close()
}, 120_000)
