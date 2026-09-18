import { expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { buildManifest } from '@demicodes/command-loader'
import { PipeBroker, RemoteHost, RemoteShellEnvironment, devicePipes } from '@demicodes/host-remote'
import {
  createRunnerWire,
  type RunnerToBackendMessage
} from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { type Command } from '@demicodes/shell'
import { memoryHostStore } from '@demicodes/shell/testing'
import { waitFor } from '@demicodes/utils'
import { nativeCommandFixture, servePipe, startRunner, type Runner } from '../../testing'

const wire = createRunnerWire(msgpackCodec)

test(
  'a native runner reports actual native and rpc leaf hints and clears them between shell statements',
  async () => {
    const home = await mkdtemp(join(tmpdir(), 'demi-hints-home-'))
    const stateDir = await mkdtemp(join(tmpdir(), 'demi-hints-state-'))
    const native = await nativeCommandFixture()
    const roots: Command[] = [{ name: 'attend', summary: 'Hint probes.', subcommands: [
      {
        name: 'native',
        summary: 'Wait for stdin locally.',
        kind: 'native',
        binding: { package: native.descriptor.id, operation: 'first' },
        runningHint: 'native: do not poll'
      },
      {
        name: 'plain',
        summary: 'Wait without a hint.',
        kind: 'native',
        binding: { package: native.descriptor.id, operation: 'first' }
      },
      {
        name: 'rpc',
        summary: 'Wait for stdin on the backend.',
        kind: 'rpc',
        runningHint: 'rpc: do not poll',
        run: () => ({ exitCode: 0 })
      },
    ] }]
    const manifest = await buildManifest(roots, { packages: [native.descriptor] })
    // The runner's claimed token names this device at the pipe routes.
    const device = { id: 'hint-device', token: 'hint-test-token' }
    const pipes = new PipeBroker()
    const host = new RemoteHost({
      defaultCwd: home,
      identity: { uid: 1, gid: 1, hostname: 'test', homeDir: home },
      store: memoryHostStore(),
      pipes: devicePipes(pipes, device.id),
    })
    const shell = new RemoteShellEnvironment({ conversation: 'test-conversation', node: 'test-session', host, commands: { manifest, resolveArtifact: native.resolveArtifact } })
    const inbound: RunnerToBackendMessage[] = []
    const calls = new Map<string, {
      stream: ReadableStream<Uint8Array>;
      close(): void
    }>()
    const server = Bun.serve({
      port: 0,
      fetch(request, bunServer) {
        const url = new URL(request.url)
        if (url.pathname === '/api/runner')
          return bunServer.upgrade(request) ? undefined : new Response(
            'no',
            { status: 400 }
          )
        const call = calls.get(url.pathname.slice('/api/pipes/'.length))
        // A relayed call's stdout is the test's own; any other pipe is the Host's.
        return call ? new Response(call.stream) : servePipe(pipes, request, device)
      },
      websocket: {
        message(ws, data) {
          const message = wire.decodeRunnerToBackend(typeof data === 'string'
            ? new Uint8Array(0)
            : new Uint8Array(data))
          inbound.push(message)
          if (message.type === 'hello') {
            host.attach((outgoing) => {
              ws.send(wire.encode(outgoing))
            })
            ws.send(wire.encode({
              type: 'claimed',
              deviceToken: device.token
            }))
            ws.send(wire.encode({ type: 'manifest', manifest }))
          } else if (message.type === 'rpc_call') {
            let close = () => {}
            let phase: 'open' | 'closed' = 'open'
            const stream = new ReadableStream<Uint8Array>({
              start(controller) {
                close = () => {
                  if (phase === 'closed') return
                  phase = 'closed'
                  controller.close()
                }
              },
              cancel() { phase = 'closed' },
            })
            calls.set(message.callId, { stream, close })
            ws.send(wire.encode({
              type: 'rpc_pipes',
              callId: message.callId,
              stdout: {
                id: message.callId,
                url: `/api/pipes/${message.callId}`
              }
            }))
            ws.send(wire.encode({ type: 'rpc_stdin_pull', callId: message.callId }))
          } else if (message.type === 'rpc_stdin'
            || message.type === 'rpc_cancel') {
            const call = calls.get(message.callId)
            if (call) {
              calls.delete(message.callId)
              call.close()
              if (message.type === 'rpc_stdin')
                ws.send(wire.encode({
                  type: 'rpc_exit',
                  callId: message.callId,
                  exitCode: 0
                }))
            }
          } else host.handleMessage(message)
        },
        close() {
          host.detach()
        },
      },
    })
    let runner: Runner | undefined
    try {
      runner = await startRunner({
        backendUrl: `http://localhost:${server.port}`,
        stateDir,
        home
      })
      for (const leaf of ['native', 'rpc']) {
        const before = inbound.length
        const started = await shell.exec({
          script: `attend ${leaf}; sleep 30`,
          timeoutMs: 100,
          agentSessionId: 'hint-session'
        })
        await waitFor(
          () => inbound.slice(before).some(
            (message) => message.type === 'job_running_hint'
              && message.hint === `${leaf}: do not poll`
          ),
          () => runner!.log.join('\n'),
          { timeoutMs: 10_000 }
        )
        const hint = inbound.slice(before).find(
          (message) => message.type === 'job_running_hint'
            && message.hint !== null
        )!
        if (hint.type !== 'job_running_hint')
          throw new Error('hint missing')
        const view = await shell.status({ commandId: started.commandId })
        expect(view.status === 'running' && view.runningHint)
          .toBe(`${leaf}: do not poll`)
        if (leaf === 'rpc')
          await waitFor(
            () => calls.size > 0,
            undefined,
            { timeoutMs: 10_000 }
          )
        await shell.write({ commandId: started.commandId, stdin: 'finish\n' })
        await waitFor(
          () => inbound.some((message) => message.type === 'job_running_hint'
            && message.invocationId === hint.invocationId
            && message.hint === null),
          () => runner!.log.join('\n'),
          { timeoutMs: 10_000 }
        )
        const continued = await shell.status({ commandId: started.commandId })
        expect(continued.status).toBe('running')
        expect('runningHint' in continued).toBe(false)
        const stopped = await shell.abort({ commandId: started.commandId })
        expect(stopped.status).toBe('aborted')
        expect('runningHint' in stopped).toBe(false)
      }

      const before = inbound.length
      const hinted = await shell.exec({
        script: 'attend rpc',
        timeoutMs: 100,
        agentSessionId: 'hint-session'
      })
      await waitFor(() => calls.size > 0, undefined, { timeoutMs: 10_000 })
      const stopped = await shell.abort({ commandId: hinted.commandId })
      expect(stopped.status).toBe('aborted')
      expect('runningHint' in stopped).toBe(false)
      await waitFor(
        () => inbound.slice(before)
          .some((message) => message.type === 'rpc_cancel'),
        undefined,
        { timeoutMs: 10_000 }
      )

      // Kill only the native client: its hint must end by socket EOF while the shell stays alive.
      const killedBefore = inbound.length
      const child = await shell.exec({
        script: `exec 9<&0; sh -c 'echo $$ > child.pid; exec attend native' <&9 & wait; sleep 30`,
        timeoutMs: 100
      })
      await waitFor(
        () => inbound.slice(killedBefore).some(
          (message) => message.type === 'job_running_hint'
            && message.hint !== null
        ),
        undefined,
        { timeoutMs: 10_000 }
      )
      const childHint = inbound.slice(killedBefore).find(
        (message) => message.type === 'job_running_hint'
          && message.hint !== null
      )!
      if (childHint.type !== 'job_running_hint')
        throw new Error('child hint missing')
      const pid = Number(new TextDecoder().decode(await host.fs.readFile(join(
        home,
        'child.pid'
      ))).trim())
      expect(Number.isSafeInteger(pid) && pid > 0).toBe(true)
      const kill = await shell.exec({
        script: `kill -KILL ${pid}`,
        timeoutMs: 10_000,
        ephemeral: true
      })
      expect(kill.status === 'exited' && kill.exitCode).toBe(0)
      await waitFor(
        () => inbound.some((message) => message.type === 'job_running_hint'
          && message.invocationId === childHint.invocationId
          && message.hint === null),
        undefined,
        { timeoutMs: 10_000 }
      )
      const alive = await shell.status({ commandId: child.commandId })
      expect(alive.status).toBe('running')
      expect('runningHint' in alive).toBe(false)
      expect((await shell.abort({ commandId: child.commandId })).status)
        .toBe('aborted')

      const withoutHint = inbound.length
      for (const script of [
        'attend native --help',
        'attend native --unknown',
        'attend'
      ]) {
        expect((await shell.exec({ script, timeoutMs: 10_000 })).status)
          .toBe('exited')
      }
      const plain = await shell.exec({ script: 'attend plain', timeoutMs: 100 })
      expect('runningHint' in plain).toBe(false)
      await shell.write({ commandId: plain.commandId, stdin: 'done\n' })
      await waitFor(
        () => host.activeJobCount === 0,
        undefined,
        { timeoutMs: 10_000 }
      )
      expect(
        inbound.slice(withoutHint).some(
          (message) => message.type === 'job_running_hint'
            && message.hint !== null
        )
      ).toBe(false)
    } finally {
      await shell.disposeAllShells()
      await runner?.stop()
      for (const call of calls.values()) call.close()
      server.stop(true)
      await rm(home, { recursive: true, force: true })
      await rm(stateDir, { recursive: true, force: true })
    }
  },
  60_000
)
