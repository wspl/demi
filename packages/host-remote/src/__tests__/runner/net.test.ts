import { mkdtemp } from 'node:fs/promises'
import { createServer, type AddressInfo } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { PipeBroker, RemoteHost, RemoteNetError, devicePipes } from '@demicodes/host-remote'
import { LocalHost } from '@demicodes/host-remote/testing'
import {
  createRunnerWire,
  type RunnerToBackendMessage
} from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { memoryHostStore } from '@demicodes/shell/testing'
import { deferred, waitFor } from '@demicodes/utils'
import { startRunner } from '../../testing'

// The runner's network streams (`runner.md` § Network streams): `net_open`
// connects a TCP socket on the device and carries it as two pipes. The fake
// backend below is the socket plus the pipe routes; the peer is a local echo
// server on a random port.

const wire = createRunnerWire(msgpackCodec)
const TOKEN = 'device-token-for-the-test'

test(
  'net streams: 1 MiB echoes byte-equal through both pipes, input EOF half-closes, refusals carry the code, connection loss closes the socket',
  async () => {
    const runnerDir = await mkdtemp(join(tmpdir(), 'demi-net-home-'))
    const stateDir = await mkdtemp(join(tmpdir(), 'demi-net-state-'))
    const payload = new Uint8Array(1024 * 1024)
    for (let i = 0; i < payload.length; i += 1) payload[i] = 97 + ((i * 13) % 26)

    // The echo peer: writes back what it receives and ends its own side once
    // the runner half-closes its write side.
    const halfClosed = deferred<void>()
    const peerClosed = deferred<void>()
    const echo = createServer((socket) => {
      socket.on('data', (data) => socket.write(data))
      socket.on('end', () => {
        halfClosed.resolve()
        socket.end()
      })
      socket.on('close', () => peerClosed.resolve())
    })
    await new Promise<void>((resolve) => echo.listen(0, '127.0.0.1', resolve))
    const echoPort = (echo.address() as AddressInfo).port

    // A peer that accepts and stays silent, for the connection-loss case.
    const sinkConnected = deferred<void>()
    const sinkClosed = deferred<void>()
    const sink = createServer((socket) => {
      socket.on('close', () => sinkClosed.resolve())
      sinkConnected.resolve()
    })
    await new Promise<void>((resolve) => sink.listen(0, '127.0.0.1', resolve))
    const sinkPort = (sink.address() as AddressInfo).port

    // A port with nothing listening, for the refused case.
    const vacant = createServer()
    await new Promise<void>((resolve) => vacant.listen(0, '127.0.0.1', resolve))
    const vacantPort = (vacant.address() as AddressInfo).port
    await new Promise<void>((resolve) => vacant.close(() => resolve()))

    // A peer that resets the connection mid-transfer: it never reads what
    // arrives and destroys the socket, so the runner sees a read error, not
    // a clean EOF.
    const reaper = createServer((socket) => {
      // Paused: the received bytes stay unread, making destroy() a reset.
      socket.pause()
      socket.on('error', () => {})
      setTimeout(() => socket.destroy(), 200)
    })
    await new Promise<void>((resolve) => reaper.listen(0, '127.0.0.1', resolve))
    const reaperPort = (reaper.address() as AddressInfo).port

    const inbound: RunnerToBackendMessage[] = []
    const uploads = new Map<string, ReturnType<typeof deferred<Uint8Array>>>()
    let socket: Bun.ServerWebSocket<unknown> | null = null
    const server = Bun.serve({
      port: 0,
      async fetch(request, bunServer) {
        const url = new URL(request.url)
        if (url.pathname === '/api/runner')
          return bunServer.upgrade(request) ? undefined : new Response('no', { status: 400 })
        if (request.method === 'GET' && url.pathname === '/api/pipes/in')
          return new Response(payload)
        if (request.method === 'GET' && url.pathname === '/api/pipes/hold')
          // Never yields bytes and never ends: the stream stays open.
          return new Response(new ReadableStream({
            start() {},
          }))
        if (request.method === 'GET' && url.pathname === '/api/pipes/flow')
          // Yields bytes forever: the transfer only ends by failure or
          // cancellation, which stops the pump via enqueue throwing.
          return new Response(new ReadableStream({
            async start(controller) {
              const chunk = new Uint8Array(64 * 1024).fill(120)
              try {
                while (true) {
                  controller.enqueue(chunk)
                  await new Promise((resolve) => setTimeout(resolve, 10))
                }
              } catch {}
            },
          }))
        if (request.method === 'PUT' && url.pathname.startsWith('/api/pipes/out')) {
          const chunks: Uint8Array[] = []
          for await (const chunk of request.body!) chunks.push(chunk)
          uploads.get(url.pathname)?.resolve(new Uint8Array(await new Blob(chunks).arrayBuffer()))
          return new Response('drained')
        }
        if (request.method === 'PUT' && url.pathname === '/api/pipes/hold-out') {
          // Consumes the upload without answering: the stream stays open.
          for await (const _chunk of request.body!) {}
          return new Response('drained')
        }
        if (request.method === 'PUT' && url.pathname === '/api/pipes/reap-out') {
          // The upload breaks when the runner fails the pipe; consuming it
          // must not surface as a test error.
          try {
            for await (const _chunk of request.body!) {}
          } catch {}
          return new Response('drained')
        }
        await request.arrayBuffer()
        return new Response('no such pipe', { status: 404 })
      },
      websocket: {
        message(ws, data) {
          const message = wire.decodeRunnerToBackend(typeof data === 'string'
            ? new Uint8Array(0)
            : new Uint8Array(data))
          inbound.push(message)
          if (message.type === 'hello') {
            socket = ws
            ws.send(wire.encode({ type: 'claimed', deviceToken: TOKEN }))
            return
          }
          remoteHost.handleMessage(message)
        },
        close(ws) {
          if (socket === ws)
            socket = null
        },
      },
    })

    const remoteHost = new RemoteHost({
      defaultCwd: runnerDir,
      identity: new LocalHost(runnerDir).identity,
      store: memoryHostStore(),
      pipes: devicePipes(new PipeBroker(), 'unused'),
    })

    const runner = await startRunner({
      backendUrl: `http://localhost:${server.port}`,
      stateDir,
      home: runnerDir,
    })
    try {
      await waitFor(
        () => socket !== null,
        () => runner.log.join('\n'),
        { timeoutMs: 10_000 }
      )
      // The hello handler above claims the device; bind the host once online.
      await waitFor(
        () => runner.statuses.includes('online'),
        () => runner.log.join('\n'),
        { timeoutMs: 10_000 }
      )
      remoteHost.attach((outgoing) => socket!.send(wire.encode(outgoing)))

      const doneFor = (pipeId: string) => inbound.find((
        m
      ): m is Extract<RunnerToBackendMessage, { type: 'pipe_done' }> => m.type
        === 'pipe_done'
        && m.pipeId === pipeId)

      // 1 MiB in through the input pipe, echoed back byte-equal through the
      // output pipe.
      uploads.set('/api/pipes/out1', deferred<Uint8Array>())
      await remoteHost.net.open({
        host: '127.0.0.1',
        port: echoPort,
        input: { id: 'in', url: '/api/pipes/in' },
        output: { id: 'out1', url: '/api/pipes/out1' },
      })
      let uploaded: Uint8Array | undefined
      void uploads.get('/api/pipes/out1')!.promise.then((bytes) => {
        uploaded = bytes
      })
      await waitFor(
        () => uploaded !== undefined,
        () => `${runner.log.join('\n')}\n${JSON.stringify(inbound.slice(-8).map((m) => m.type))}`,
        { timeoutMs: 15_000 }
      )
      expect(uploaded!.byteLength).toBe(payload.byteLength)
      expect(uploaded!.every((byte, i) => byte === payload[i])).toBe(true)
      await waitFor(
        () => doneFor('in') !== undefined && doneFor('out1') !== undefined,
        () => runner.log.join('\n'),
        { timeoutMs: 15_000 }
      )
      expect(doneFor('in')).toEqual({ type: 'pipe_done', pipeId: 'in', ok: true })
      expect(doneFor('out1')).toEqual({ type: 'pipe_done', pipeId: 'out1', ok: true })
      // Input EOF shut the socket's write side; the echo server saw the
      // half-close and ended, which ended the output pipe.
      await halfClosed.promise
      await peerClosed.promise

      // A port with nothing listening answers `refused`.
      const refusal = remoteHost.net.open({
        host: '127.0.0.1',
        port: vacantPort,
        input: { id: 'in2', url: '/api/pipes/in' },
        output: { id: 'out2', url: '/api/pipes/out2' },
      })
      await expect(refusal).rejects.toBeInstanceOf(RemoteNetError)
      await refusal.catch((error) => {
        expect((error as RemoteNetError).code).toBe('refused')
      })

      // A peer reset mid-transfer fails the output pipe: the backend sees
      // pipe_done ok=false, not a clean EOF.
      await remoteHost.net.open({
        host: '127.0.0.1',
        port: reaperPort,
        input: { id: 'flow-in', url: '/api/pipes/flow' },
        output: { id: 'reap-out', url: '/api/pipes/reap-out' },
      })
      await waitFor(
        () => {
          const done = doneFor('reap-out')
          return done !== undefined && done.ok === false
        },
        () => `${runner.log.join('\n')}\n${JSON.stringify(inbound.slice(-8).map((m) => m.type === 'pipe_done' ? m : { type: m.type }))}`,
        { timeoutMs: 15_000 }
      )
      expect(doneFor('reap-out')!.ok).toBe(false)
      expect(doneFor('reap-out')!.error).toBeDefined()

      // An open stream's socket does not outlive the runner connection: the
      // silent peer observes the close.
      await remoteHost.net.open({
        host: '127.0.0.1',
        port: sinkPort,
        input: { id: 'hold-in', url: '/api/pipes/hold' },
        output: { id: 'hold-out', url: '/api/pipes/hold-out' },
      })
      await sinkConnected.promise
      await runner.stop()
      // The peer observes the socket closing; no stream outlives the
      // connection that opened it.
      await sinkClosed.promise
    } finally {
      await runner.stop()
      server.stop(true)
      echo.close()
      sink.close()
      reaper.close()
    }
  },
  60_000,
)

test(
  'net streams: a backend that fails the output pipe mid-upload ends the upload without a peer EOF',
  async () => {
    const runnerDir = await mkdtemp(join(tmpdir(), 'demi-net-fail-home-'))
    const stateDir = await mkdtemp(join(tmpdir(), 'demi-net-fail-state-'))

    // The peer accepts, speaks once, and never EOFs: only the backend's
    // pipe failure may end the stream (`runner.md` § Network streams: a pipe
    // failing closes the socket).
    const spoke = deferred<void>()
    const peerClosed = deferred<void>()
    const peer = createServer((socket) => {
      socket.write('the peer speaks and then holds the connection open\n')
      spoke.resolve()
      socket.on('error', () => {})
      socket.on('close', () => peerClosed.resolve())
    })
    await new Promise<void>((resolve) => peer.listen(0, '127.0.0.1', resolve))
    const peerPort = (peer.address() as AddressInfo).port

    const inbound: RunnerToBackendMessage[] = []
    let socket: Bun.ServerWebSocket<unknown> | null = null
    const server = Bun.serve({
      port: 0,
      async fetch(request, bunServer) {
        const url = new URL(request.url)
        if (url.pathname === '/api/runner')
          return bunServer.upgrade(request) ? undefined : new Response('no', { status: 400 })
        if (request.method === 'GET' && url.pathname === '/api/pipes/hold')
          // Never yields bytes and never ends: the stream stays open.
          return new Response(new ReadableStream({
            start() {},
          }))
        if (request.method === 'PUT' && url.pathname === '/api/pipes/fail-out') {
          // The backend fails the pipe while the upload is still streaming:
          // the answer leaves the request body unread.
          await Bun.sleep(100)
          return new Response('pipe failed: visitor connection ended', { status: 409 })
        }
        await request.arrayBuffer()
        return new Response('no such pipe', { status: 404 })
      },
      websocket: {
        message(ws, data) {
          const message = wire.decodeRunnerToBackend(typeof data === 'string'
            ? new Uint8Array(0)
            : new Uint8Array(data))
          inbound.push(message)
          if (message.type === 'hello') {
            socket = ws
            ws.send(wire.encode({ type: 'claimed', deviceToken: TOKEN }))
            return
          }
          remoteHost.handleMessage(message)
        },
        close(ws) {
          if (socket === ws)
            socket = null
        },
      },
    })

    const remoteHost = new RemoteHost({
      defaultCwd: runnerDir,
      identity: new LocalHost(runnerDir).identity,
      store: memoryHostStore(),
      pipes: devicePipes(new PipeBroker(), 'unused'),
    })

    const runner = await startRunner({
      backendUrl: `http://localhost:${server.port}`,
      stateDir,
      home: runnerDir,
    })
    try {
      await waitFor(
        () => socket !== null,
        () => runner.log.join('\n'),
        { timeoutMs: 10_000 }
      )
      await waitFor(
        () => runner.statuses.includes('online'),
        () => runner.log.join('\n'),
        { timeoutMs: 10_000 }
      )
      remoteHost.attach((outgoing) => socket!.send(wire.encode(outgoing)))

      const doneFor = (pipeId: string) => inbound.find((
        m
      ): m is Extract<RunnerToBackendMessage, { type: 'pipe_done' }> => m.type
        === 'pipe_done'
        && m.pipeId === pipeId)

      await remoteHost.net.open({
        host: '127.0.0.1',
        port: peerPort,
        input: { id: 'hold-in', url: '/api/pipes/hold' },
        output: { id: 'fail-out', url: '/api/pipes/fail-out' },
      })
      await spoke.promise
      // The 409 fails the upload without the peer's EOF, the stream cancels,
      // the socket closes, and both pipe ends report.
      await waitFor(
        () => doneFor('hold-in') !== undefined && doneFor('fail-out') !== undefined,
        () => runner.log.join('\n'),
        { timeoutMs: 15_000 }
      )
      expect(doneFor('fail-out')!.ok).toBe(false)
      expect(doneFor('fail-out')!.error).toBeDefined()
      expect(doneFor('hold-in')!.ok).toBe(false)
      await peerClosed.promise
    } finally {
      await runner.stop()
      server.stop(true)
      peer.close()
    }
  },
  60_000,
)
