import { expect, test } from 'bun:test'
import { mkdtemp, readFile, writeFile, realpath } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { buildManifest, type Manifest } from '@demicodes/command-loader'
import { RemoteHost, RemoteShellEnvironment } from '@demicodes/host-remote'
import { createRunnerWire } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { runtimeModule, type Command } from '@demicodes/shell'
import { memoryHostStore } from '@demicodes/shell/testing'
import { waitFor } from '@demicodes/utils'
import { packedRunner, startTxikiRunner } from '../testing'

async function fixture(label: string, commands: Command[] = []) {
  const home = await realpath(await mkdtemp(join(tmpdir(), 'demi-native-home-')))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-native-state-'))
  const roots: Command[] = [{ name: 'demi', summary: label, subcommands: [
    {
      name: 'where',
      summary: 'Invocation context',
      kind: 'runtime',
      module: runtimeModule(
        `export default async ctx => { await ctx.stdout(JSON.stringify({label:${JSON.stringify(label)},cwd:ctx.cwd,value:ctx.env.PROBE})); return {exitCode:0}; }`
      )
    },
    {
      name: 'echo',
      summary: 'Byte stream',
      kind: 'runtime',
      module: runtimeModule(
        'export default async ctx => { for await(const b of ctx.stdin) await ctx.stdout(b); return {exitCode:0}; }'
      )
    },
    {
      name: 'spin',
      summary: 'Interruptible worker',
      kind: 'runtime',
      module: runtimeModule(
        'export default async ctx => { await ctx.stdout("started"); while(true){} }'
      )
    },
    ...commands,
  ] }]
  const manifest = await buildManifest(roots, { transpile: source => source })
  const wire = createRunnerWire(msgpackCodec)
  const host = new RemoteHost({
    defaultCwd: home,
    identity: { uid: 1, gid: 1, hostname: label, homeDir: home },
    store: memoryHostStore()
  })
  const shell = new RemoteShellEnvironment({ host })
  let sendManifest: (value: Manifest) => void
  const server = Bun.serve(
    { port: 0, fetch: (request, server) => server.upgrade(request) ? undefined : new Response(
      'missing',
      { status: 404 }
    ), websocket: {
      message(ws, data) {
        const message = wire.decodeRunnerToBackend(new Uint8Array(data as Buffer))
        if (message.type === 'hello') {
          host.attach(message => {
            ws.send(wire.encode(message))
          })
          sendManifest = value => {
            ws.send(wire.encode({ type: 'manifest', manifest: value }))
          }
          ws.send(wire.encode({ type: 'hello_ok', deviceId: label }))
          ws.send(wire.encode({ type: 'manifest', manifest }))
        } else host.handleMessage(message)
      }, close() {
        host.detach()
      },
    } }
  )
  const runner = await startTxikiRunner({
    backendUrl: `http://localhost:${server.port}`,
    home,
    stateDir,
    deviceToken: label
  })
  await waitFor(
    () => runner.log.some(line => line.includes('installed: demi')),
    () => runner.log.join('\n')
  )
  return { home, stateDir, host, shell, runner, async replaceRoot() {
    const updated = await buildManifest(
      [{ ...roots[0]!, name: 'replacement' }],
      { transpile: source => source }
    )
    sendManifest(updated)
    await waitFor(
      () => runner.log.some(line => line.includes('installed: replacement')),
      () => runner.log.join('\n')
    )
  }, backendUrl: `http://localhost:${server.port}`, async close() {
    await runner.stop();
    server.stop(true)
  } }
}

test(
  'native commands route to their own runner and keep cwd/env and binary streams independent',
  async () => {
    const a = await fixture('A'), b = await fixture('B')
    try {
      const calls = await Promise.all([
        a.shell.exec({ script: 'PROBE=alpha demi where', timeoutMs: 10_000 }),
        b.shell.exec({ script: 'PROBE=beta demi where', timeoutMs: 10_000 })
      ])
      expect(JSON.parse(calls[0]!.stdout.delta)).toEqual({
        label: 'A',
        cwd: a.home,
        value: 'alpha'
      })
      expect(JSON.parse(calls[1]!.stdout.delta)).toEqual({
        label: 'B',
        cwd: b.home,
        value: 'beta'
      })
      const bytes = new Uint8Array(3 * 1024 * 1024).map((_, i) => i % 256)
      await writeFile(join(a.home, 'input'), bytes)
      const echoed = await a.shell.exec({
        script: 'cat input | demi echo > output',
        timeoutMs: 10_000
      })
      expect(echoed.status === 'exited' && echoed.exitCode).toBe(0)
      expect(new Uint8Array(await readFile(join(a.home, 'output'))))
        .toEqual(bytes)
      const capture = await a.shell.exec({
        script: 'printf "%s\\n%s\\n" "$DEMI_RUNNER_ENDPOINT" "$DEMI_CONTEXT_ID" > context; sleep 30',
        timeoutMs: 50
      })
      const [endpoint, context] = (await readFile(
        join(a.home, 'context'),
        'utf8'
      )).trim().split('\n')
      const other = JSON.parse(await readFile(
        join(b.stateDir, 'active.json'),
        'utf8'
      ))
      expect(endpoint).not.toBe(other.endpoint)
      const client = join(
        (await packedRunner()).replace(/\/demi-runner$/, ''),
        'demi'
      )
      const wrong = Bun.spawnSync(
        [client, 'where'],
        {
          env: {
            ...process.env,
            DEMI_RUNNER_ENDPOINT: other.endpoint,
            DEMI_CONTEXT_ID: context
          },
          stdout: 'pipe',
          stderr: 'pipe'
        }
      )
      expect(wrong.exitCode).toBe(1)
      expect(wrong.stderr.toString()).toContain('not live on this runner')
      await a.shell.abort({ commandId: capture.commandId })
      const stale = Bun.spawnSync(
        [client, 'where'],
        {
          env: {
            ...process.env,
            DEMI_RUNNER_ENDPOINT: endpoint,
            DEMI_CONTEXT_ID: context
          },
          stdout: 'pipe',
          stderr: 'pipe'
        }
      )
      expect(stale.exitCode).toBe(1)
    } finally {
      await a.close();
      await b.close()
    }
  },
  30_000
)

test(
  'a CPU-bound command can be cancelled without blocking the runner; duplicate installation is refused',
  async () => {
    const f = await fixture('isolated')
    try {
      const started = await f.shell.exec({ script: 'demi spin', timeoutMs: 100 })
      expect(started.status).toBe('running')
      expect(await f.host.fs.exists(f.home)).toBe(true)
      const duplicate = Bun.spawnSync(
        [await packedRunner(), 'run', '--backend', f.backendUrl],
        {
          env: { ...process.env, DEMI_HOME: f.stateDir },
          stdout: 'pipe',
          stderr: 'pipe'
        }
      )
      expect(duplicate.exitCode).toBe(1)
      expect(duplicate.stderr.toString()).toContain('already active')
      expect(
        (await f.shell.abort({ commandId: started.commandId })).status
      ).toBe('aborted')
      const next = await f.shell.exec({ script: 'demi --help', timeoutMs: 10_000 })
      expect(next.status === 'exited' && next.exitCode).toBe(0)
    } finally {
      await f.close()
    }
  },
  15_000
)

test(
  'workers preserve output and report command failures and invalid exit codes',
  async () => {
    const f = await fixture('worker-results', [{
      name: 'result',
      summary: 'Worker completion cases',
      kind: 'runtime',
      module: runtimeModule(`
      export default async context => {
        await context.stdout('command output');
        await context.stderr('command diagnostic');

        if (context.env.RESULT === 'error') {
          throw new Error('command failed');
        }
        if (context.env.RESULT === 'invalid') {
          return { exitCode: 256 };
        }
        return { exitCode: 17 };
      }
    `),
    }])

    try {
      const completed = await f.shell.exec({
        script: 'demi result',
        timeoutMs: 10_000
      })
      expect(completed.status === 'exited' && completed.exitCode).toBe(17)
      expect(completed.stdout.delta).toBe('command output')
      expect(completed.stderr.delta).toBe('command diagnostic')

      const failed = await f.shell.exec({
        script: 'RESULT=error demi result',
        timeoutMs: 10_000
      })
      expect(failed.status === 'exited' && failed.exitCode).toBe(1)
      expect(failed.stderr.delta).toContain('command failed')

      const invalid = await f.shell.exec({
        script: 'RESULT=invalid demi result',
        timeoutMs: 10_000
      })
      expect(invalid.status === 'exited' && invalid.exitCode).toBe(1)
      expect(invalid.stderr.delta).toContain('invalid command exit code')
    } finally {
      await f.close()
    }
  },
  15_000
)


test(
  'an active job keeps its manifest and root aliases after a manifest update',
  async () => {
    const f = await fixture('pinned')
    try {
      const started = await f.shell.exec({
        script: 'touch ready; while [ ! -f proceed ]; do sleep 0.01; done; PROBE=old demi where > result',
        timeoutMs: 50
      })
      expect(started.status).toBe('running')
      await f.replaceRoot()
      await writeFile(join(f.home, 'proceed'), '')
      await waitFor(() => {
        try {
          return JSON.parse(readFileSync(join(f.home, 'result'), 'utf8'))
            .value === 'old'
        } catch {
          return false
        }
      })
      while ((await f.shell.status({ commandId: started.commandId })).status === 'running') await Bun.sleep(10)
      const next = await f.shell.exec({
        script: 'PROBE=new replacement where',
        timeoutMs: 10_000
      })
      expect(JSON.parse(next.stdout.delta).value).toBe('new')
    } finally {
      await f.close()
    }
  },
  15_000
)
