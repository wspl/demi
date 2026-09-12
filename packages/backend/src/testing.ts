// Integration fixtures use the production registry, pipes, packed runner and shell.
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Hono } from 'hono'
import { createBunWebSocket } from 'hono/bun'
import { buildManifest, inProcessRpc } from '@demicodes/command-loader'
import { RemoteShellEnvironment } from '@demicodes/host-remote'
import { startTxikiRunner } from '@demicodes/runner/testing'
import {
  type CommandStorage,
  type CommandRegistry,
  type Host,
  type ShellEnvironmentOptions
} from '@demicodes/shell'
import { memoryCommandStorage } from '@demicodes/shell/testing'
import { waitFor } from '@demicodes/utils'
import { transpileCommandModule } from './conversation/command-manifest'
import { runnerSocketRoutes } from './http/runner-socket'
import { pipeRoutes } from './http/pipes'
import { generateDeviceToken, hashDeviceToken } from './runner/claim-codes'
import { PipeBroker } from './runner/pipes'
import { RunnerRegistry } from './runner/registry'
import { LocalControlService } from './storage/control'
import { openSqliteDatabase } from './storage/database'
import { migrate, CONTROL_MIGRATIONS } from './storage/migrations'

const fixtures = new WeakMap<Host, Promise<RunnerFixture>>()
interface RunnerFixture {
  registry: RunnerRegistry
  deviceId: string
  commands: Map<string, CommandRegistry>
  users: number
  close(): Promise<void>
}

export async function runnerShell(options: ShellEnvironmentOptions & {
  host: Host;
  commands: CommandRegistry;
  agentSessionId?: string;
  commandStorage?: (signal?: AbortSignal) => CommandStorage
}) {
  const { host, commands, agentSessionId = 'test-session', commandStorage, ...shell } = options
  const storage = memoryCommandStorage()
  let pending = fixtures.get(host)
  if (!pending) {
    pending = createFixture(host, commands)
    fixtures.set(host, pending)
    pending.catch(() => fixtures.delete(host))
  }
  const fixture = await pending
  fixture.users++
  fixture.commands.set(agentSessionId, commands)
  const remote = fixture.registry.hostFor({
    deviceId: fixture.deviceId,
    path: host.defaultCwd
  }, agentSessionId, host.store)
  const environment = new RemoteShellEnvironment({
    ...shell,
    initialEnv: {
      DEMI_SESSION_ID: agentSessionId,
      ...shell.initialEnv
    },
    host: remote,
    commandStorage: commandStorage ?? (() => storage)
  })
  const exec = environment.exec.bind(environment)
  const sessions = new Set([agentSessionId])
  environment.exec = input => {
    const sessionId = input.agentSessionId ?? agentSessionId
    sessions.add(sessionId)
    fixture.commands.set(sessionId, commands)
    return exec(input)
  }
  const dispose = environment.disposeAllShells.bind(environment)
  let disposed = false
  environment.disposeAllShells = async () => {
    if (disposed)
      return
    disposed = true
    await dispose()
    for (const id of sessions)
      fixture.commands.delete(id)
    if (--fixture.users === 0) {
      fixtures.delete(host);
      await fixture.close()
    }
  }
  return environment
}

async function createFixture(
  host: Host,
  initialCommands: CommandRegistry
): Promise<RunnerFixture> {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  const user = (await control.createMaster(
    { email: 'test@example.test', passwordHash: '' }
  ))!
  const token = generateDeviceToken()
  const device = await control.createDevice({
    userId: user.id,
    name: 'test',
    platform: 'test',
    tokenHash: hashDeviceToken(token)
  })
  const pipes = new PipeBroker()
  const roots = initialCommands.list()
  const commands = new Map<string, CommandRegistry>()
  const manifest = await buildManifest(
    roots,
    { transpile: transpileCommandModule }
  )
  const registry = new RunnerRegistry({
    control,
    pipes,
    pingIntervalMs: 0,
    manifest: async () => manifest,
    rpc: async (call, io, execution) => {
      if (!execution.commandStorage)
        throw new Error('rpc job has no command storage')
      const result = await inProcessRpc(
        commands.get(call.agentSessionId)!.list(),
        {
          host: execution.host,
          storage: execution.commandStorage.withSignal(io.signal)
        }
      )({
        root: call.root,
        path: call.path,
        argv: call.argv,
        args: call.args,
        json: call.json,
        stdin: io.stdin?.stream() ?? null,
        cwd: call.cwd,
        env: call.env,
        io: io.commandIO(),
        signal: io.signal,
        stdinStream: io.stdinStream,
      })
      return result.exitCode
    },
  })
  const { upgradeWebSocket, websocket } = createBunWebSocket()
  const app = new Hono()
  app.route('/api/runner', runnerSocketRoutes({ registry, upgradeWebSocket }))
  app.route('/api/pipes', pipeRoutes({ control, broker: pipes }))
  const server = Bun.serve({ port: 0, idleTimeout: 0, fetch: app.fetch, websocket })
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-test-runner-'))
  let runner: Awaited<ReturnType<typeof startTxikiRunner>> | undefined
  const close = async () => {
    await runner?.stop()
    await registry.close()
    pipes.close()
    server.stop(true)
    db.close()
  }
  try {
    runner = await startTxikiRunner({
      backendUrl: `http://localhost:${server.port}`,
      stateDir,
      home: host.defaultCwd,
      deviceToken: token
    })
    await registry.whenOnline(device.id)
    await waitFor(
      () => runner!.log.some(line => line.includes(' installed:')),
      () => runner!.log.join('\n'),
      { timeoutMs: 15_000 }
    )
    return { registry, deviceId: device.id, commands, users: 0, close }
  } catch (error) {
    await close();
    throw error
  }
}

export function runnerShellFactory(ctx: {
  agentSessionId: string;
  host: Host;
  commands: CommandRegistry;
  shell: ShellEnvironmentOptions;
  commandStorage(signal?: AbortSignal): CommandStorage
}) {
  return runnerShell({
    ...ctx.shell,
    host: ctx.host,
    commands: ctx.commands,
    agentSessionId: ctx.agentSessionId,
    commandStorage: ctx.commandStorage
  })
}

import { type Command } from '@demicodes/shell'
import { delay, utf8Lines } from '@demicodes/utils'
import { z } from 'zod'

export function probeCommand(): Command {
  return {
    name: 'probe',
    summary: 'Test probes: hold for a while, or echo the first line of the live stdin.',
    subcommands: [
      {
        name: 'hold',
        summary: 'Wait `ms` milliseconds (aborted with the command).',
        input: { ms: z.coerce.number() },
        positionals: ['ms'],
        kind: 'rpc',
        run: async ({ parsed, signal }) => {
          const ms = parsed.values.ms as number
          const held = delay(ms)
          const aborted = new Promise<'aborted'>(
            (resolve) => signal.addEventListener(
              'abort',
              () => resolve('aborted'),
              { once: true }
            )
          )
          return (await Promise.race([
            held.then(() => 'held' as const),
            aborted
          ])) === 'aborted' ? { exitCode: 130 } : { exitCode: 0 }
        },
      },
      {
        name: 'stdin',
        summary: 'Print the first line written to the running command (`--delay` waits before printing).',
        input: { delay: z.coerce.number().optional() },
        kind: 'rpc',
        run: async ({ parsed, stdinStream, io }) => {
          for await (const line of utf8Lines(stdinStream)) {
            const wait = parsed.values.delay as number | undefined
            if (wait)
              await delay(wait)
            await io.stdout(line)
            return { exitCode: 0 }
          }
          return { exitCode: 1 }
        },
      },
    ],
  }
}
