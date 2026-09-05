import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { createRunnerWire, type RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { deferred, waitFor } from '@demicodes/utils'
import { startTinyjsRunner } from '../testing'

// The runner's ends of pipes (`runner.md` § Pipes): a job started with
// `stdin` GETs the origin-relative URL with its device token into the job's
// fd 0; one started with `stdout` PUTs the job's full stdout as the job
// writes it; each end reports `pipe_done`. The fake backend below is the
// socket plus the pipe routes, and never holds a body.

const wire = createRunnerWire(msgpackCodec)
const TOKEN = 'device-token-for-the-test'

test('job pipes: stdin is fetched into the job, stdout is streamed out as it is written, both report pipe_done', async () => {
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-pipes-home-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-pipes-state-'))
  // Well past the tee's stream buffer and the view: only a stream carries it whole.
  const payload = new Uint8Array(3 * 1024 * 1024)
  for (let i = 0; i < payload.length; i += 1) payload[i] = 97 + ((i * 7) % 26)

  const inbound: RunnerToBackendMessage[] = []
  const uploads = new Map<string, ReturnType<typeof deferred<Uint8Array>>>()
  const authorizations: string[] = []
  let socket: Bun.ServerWebSocket<unknown> | null = null
  const server = Bun.serve({
    port: 0,
    async fetch(request, bunServer) {
      const url = new URL(request.url)
      if (url.pathname === '/api/runner') return bunServer.upgrade(request) ? undefined : new Response('no', { status: 400 })
      authorizations.push(request.headers.get('authorization') ?? '')
      if (request.method === 'GET' && url.pathname === '/api/pipes/in') return new Response(payload)
      if (request.method === 'PUT' && url.pathname.startsWith('/api/pipes/out')) {
        // Consumed as it streams: the body is never buffered by the server as a whole until the end.
        const chunks: Uint8Array[] = []
        for await (const chunk of request.body!) chunks.push(chunk)
        uploads.get(url.pathname)?.resolve(new Uint8Array(await new Blob(chunks).arrayBuffer()))
        return new Response('drained')
      }
      // Drained before refusing, so the runner sees the status rather than a reset.
      await request.arrayBuffer()
      return new Response('no such pipe', { status: 404 })
    },
    websocket: {
      message(ws, data) {
        const message = wire.decodeRunnerToBackend(typeof data === 'string' ? new Uint8Array(0) : new Uint8Array(data))
        inbound.push(message)
        if (message.type === 'hello') {
          socket = ws
          ws.send(wire.encode({ type: 'claimed', deviceToken: TOKEN }))
        }
      },
    },
  })

  const runner = await startTinyjsRunner({ backendUrl: `http://localhost:${server.port}`, stateDir, home: runnerDir })
  await waitFor(() => runner.statuses.includes('online'), () => runner.log.join('\n'), { timeoutMs: 10_000 })
  const send = (message: Parameters<typeof wire.encode>[0]) => socket!.send(wire.encode(message))
  const doneFor = (pipeId: string) => inbound.find((m): m is Extract<RunnerToBackendMessage, { type: 'pipe_done' }> => m.type === 'pipe_done' && m.pipeId === pipeId)
  const exitOf = (jobId: string) => inbound.find((m): m is Extract<RunnerToBackendMessage, { type: 'job_exit' }> => m.type === 'job_exit' && m.jobId === jobId)

  // In through fd 0, out through fd 1: `tr` upper-cases the 3 MB as it streams.
  uploads.set('/api/pipes/out1', deferred<Uint8Array>())
  send({
    type: 'job_start',
    jobId: 'j1',
    script: 'tr a-z A-Z',
    cwd: runnerDir,
    env: { PATH: process.env.PATH ?? '/usr/bin:/bin', HOME: runnerDir },
    stdin: { id: 'in', url: '/api/pipes/in' },
    stdout: { id: 'out1', url: '/api/pipes/out1' },
  })
  const uploaded = await uploads.get('/api/pipes/out1')!.promise
  await waitFor(() => doneFor('in') !== undefined && doneFor('out1') !== undefined && exitOf('j1') !== undefined, () => runner.log.join('\n'), { timeoutMs: 20_000 })
  expect(doneFor('in')).toEqual({ type: 'pipe_done', pipeId: 'in', ok: true })
  expect(doneFor('out1')).toEqual({ type: 'pipe_done', pipeId: 'out1', ok: true })
  expect(uploaded.byteLength).toBe(payload.byteLength)
  expect(uploaded.every((byte, i) => byte === payload[i]! - 32)).toBe(true)
  const exit = exitOf('j1')!
  expect(exit.exitCode).toBe(0)
  expect(exit.output?.stdoutBytes).toBe(payload.byteLength)
  // The view still crossed the socket, bounded as ever.
  expect(inbound.filter((m) => m.type === 'job_output' && m.jobId === 'j1').reduce((n, m) => n + (m.type === 'job_output' ? m.bytes.byteLength : 0), 0)).toBeLessThanOrEqual(32 * 1024)
  expect(new Set(authorizations)).toEqual(new Set([`Bearer ${TOKEN}`]))

  // A refused stdout end is reported and released: the job still runs to its end instead of blocking on a reader that left.
  send({
    type: 'job_start',
    jobId: 'j2',
    script: 'head -c 2000000 /dev/zero; echo done >&2',
    cwd: runnerDir,
    env: { PATH: process.env.PATH ?? '/usr/bin:/bin', HOME: runnerDir },
    stdout: { id: 'gone', url: '/api/pipes/gone' },
  })
  await waitFor(() => doneFor('gone') !== undefined && exitOf('j2') !== undefined, () => runner.log.join('\n'), { timeoutMs: 20_000 })
  expect(doneFor('gone')).toEqual({ type: 'pipe_done', pipeId: 'gone', ok: false, error: 'pipe refused (404): no such pipe' })
  expect(exitOf('j2')?.exitCode).toBe(0)
  expect(exitOf('j2')?.output?.stdoutBytes).toBe(2_000_000)

  // A refused stdin end closes the job's stdin, so a reader of it ends rather than waits.
  send({
    type: 'job_start',
    jobId: 'j3',
    script: 'wc -c',
    cwd: runnerDir,
    env: { PATH: process.env.PATH ?? '/usr/bin:/bin', HOME: runnerDir },
    stdin: { id: 'missing', url: '/api/pipes/missing' },
  })
  await waitFor(() => doneFor('missing') !== undefined && exitOf('j3') !== undefined, () => runner.log.join('\n'), { timeoutMs: 20_000 })
  expect(doneFor('missing')?.ok).toBe(false)
  expect(new TextDecoder().decode(exitOf('j3')?.output?.stdoutTail).trim()).toBe('0')

  await runner.stop()
  server.stop(true)
}, 90_000)
