import { describe, expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { buildManifest, createLoader, inMemorySource, inProcessRpc, rootPaths } from '@demicodes/command-loader'
import { createDemiCommand } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { VirtualHost, scopedFsBackend } from '@demicodes/host-virtual'
import { AgentSessionCommandStorage, type Command, type ShellCommandStatus } from '@demicodes/shell'
import { delay } from '@demicodes/utils'
import { z } from 'zod'
import { HostlessEnvironment } from '@demicodes/host-virtual'
import { HOSTLESS_IDENTITY, transpileCommandModule } from '../conversation/hostless-shell'
import { HOSTLESS_HOME, HOSTLESS_NAMESPACE } from '../conversation/scoped-transport'

/**
 * The hostless shell end to end: tinybash over a VirtualHost, `demi file`
 * as runtime modules, `demi todo` as in-process rpc, refusals, session
 * state, steering and abort, binary output.
 */

/** A root with an rpc leaf that waits: for stdin lines, or for the abort signal. */
function attendRoot(): Command {
  return {
    name: 'scout',
    summary: 'Test root.',
    subcommands: [
      {
        name: 'attend',
        kind: 'rpc',
        summary: 'Echo steer lines until stdin closes or the signal fires.',
        input: { greeting: z.string().optional() },
        run: async ({ parsed, io, signal, stdinStream }) => {
          await io.stdout(`${parsed.values.greeting ?? 'ready'}\n`)
          const done = new Promise<number>((resolve) => signal.addEventListener('abort', () => resolve(130), { once: true }))
          const reading = (async () => {
            for await (const chunk of stdinStream) {
              await io.stdout(`steer: ${new TextDecoder().decode(chunk)}`)
              if (new TextDecoder().decode(chunk).startsWith('stop')) return 0
            }
            return 0
          })()
          return { exitCode: await Promise.race([done, reading]) }
        },
      },
    ],
  }
}

async function world() {
  const dir = mkdtempSync(join(tmpdir(), 'hostless-'))
  const local = new LocalHost(dir, { storeRoot: join(dir, '.store') })
  const host = new VirtualHost({
    backend: scopedFsBackend(join(dir, 'fs'), local.fs),
    store: local.store,
    defaultCwd: HOSTLESS_HOME,
    directories: HOSTLESS_NAMESPACE,
  })
  await host.ensureLayout()
  const roots: Command[] = [createDemiCommand(), attendRoot()]
  const manifest = await buildManifest(roots, { transpile: transpileCommandModule })
  const loader = await createLoader({
    source: inMemorySource(manifest),
    host,
    rpc: inProcessRpc(roots, { storage: new AgentSessionCommandStorage(host.store, 'session-1'), host }),
  })
  let shells = 0
  let commands = 0
  const env = new HostlessEnvironment({
    host,
    roots: rootPaths(loader.roots),
    dispatch: (root, argv, io) => loader.dispatch(root, argv, io),
    home: HOSTLESS_HOME,
    namespace: HOSTLESS_NAMESPACE,
    identity: HOSTLESS_IDENTITY,
    initialEnv: { PATH: '/usr/bin:/bin' },
    shellIdFactory: () => `shell-${++shells}`,
    commandIdFactory: () => `cmd-${++commands}`,
  })
  const exec = (script: string, extra: { timeoutMs?: number; shellId?: string; ephemeral?: boolean; cwd?: string } = {}) =>
    env.exec({ script, timeoutMs: 5_000, agentSessionId: 'session-1', ...extra })
  return { dir, host, env, exec, dispose: () => rmSync(dir, { recursive: true, force: true }) }
}

function exited(status: ShellCommandStatus): Extract<ShellCommandStatus, { status: 'exited' }> {
  if (status.status !== 'exited') throw new Error(`expected exited, got ${status.status}`)
  return status
}

describe('hostless conversation', () => {
  test('demi file through heredocs, builtin pipelines, sequences; state persists in the default shell', async () => {
    const w = await world()
    try {
      const created = exited(await w.exec("mkdir src && cd src && demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF"))
      expect(created.exitCode).toBe(0)
      expect(created.stdout.delta).toBe('Created notes.md\n')

      const piped = exited(await w.exec('pwd; demi file read notes.md | grep -n a | sort -r; echo "$HOME"'))
      expect(piped.stdout.delta).toBe(`${HOSTLESS_HOME}/src\n3:gamma\n2:beta\n1:alpha\n${HOSTLESS_HOME}\n`)

      const edited = exited(await w.exec("demi file edit notes.md --old beta --new delta && cat notes.md > ../copy.txt && wc -l < ../copy.txt"))
      expect(edited.stdout.delta).toBe('Edited notes.md\n3\n')
      expect(new TextDecoder().decode(await w.host.fs.readFile(`${HOSTLESS_HOME}/copy.txt`))).toBe('alpha\ndelta\ngamma\n')

      const failed = exited(await w.exec('demi file create notes.md <<EOF\nagain\nEOF\necho after'))
      expect(failed.stderr.delta).toBe('File already exists: notes.md\n')
      expect(failed.stdout.delta).toBe('after\n')
      expect(failed.exitCode).toBe(0)
    } finally {
      w.dispose()
    }
  })

  test('demi todo is an rpc leaf: storage lives with the session', async () => {
    const w = await world()
    try {
      const added = exited(await w.exec('demi todo add "draft the outline" && demi todo add "run the suite"'))
      expect(added.exitCode).toBe(0)
      const listed = exited(await w.exec('demi todo list --json'))
      expect(JSON.parse(listed.stdout.delta)).toMatchObject({ todos: [{ text: 'draft the outline' }, { text: 'run the suite' }] })
    } finally {
      w.dispose()
    }
  })

  test('a script outside the subset runs nothing and says so; a bare group prints help', async () => {
    const w = await world()
    try {
      const refused = exited(await w.exec('echo started > marker.txt; python3 -V'))
      expect(refused.exitCode).toBe(2)
      expect(refused.stdout.delta).toBe('')
      expect(refused.stderr.delta).toBe('tinybash: line 1: python3: no such program here; a machine\n')
      expect(await w.host.fs.exists(`${HOSTLESS_HOME}/marker.txt`)).toBe(false)

      const help = exited(await w.exec('demi file'))
      expect(help.exitCode).toBe(0)
      expect(help.stdout.delta).toContain('demi file read')
    } finally {
      w.dispose()
    }
  })

  test('the observation window, steering through shell_write, and abort', async () => {
    const w = await world()
    try {
      const started = await w.exec('scout attend --greeting hello', { timeoutMs: 50 })
      expect(started.status).toBe('running')
      expect(started.stdout.delta).toBe('hello\n')
      const steered = await w.env.write({ commandId: started.commandId, stdin: 'turn left\n' })
      expect(steered.status).toBe('running')
      await new Promise((resolve) => setTimeout(resolve, 20))
      const polled = await w.env.status({ commandId: started.commandId })
      expect(polled.stdout.delta).toBe('steer: turn left\n')
      const aborted = await w.env.abort({ commandId: started.commandId })
      expect(aborted.status).toBe('aborted')

      const second = await w.exec('scout attend', { timeoutMs: 50 })
      expect(second.status).toBe('running')
      await w.env.write({ commandId: second.commandId, stdin: 'stop\n' })
      await new Promise((resolve) => setTimeout(resolve, 20))
      const finished = exited(await w.env.status({ commandId: second.commandId }))
      expect(finished.exitCode).toBe(0)
      expect(finished.stdout.delta).toBe('steer: stop\n')
    } finally {
      w.dispose()
    }
  })

  test('a binary final stream is carried as bytes with a placeholder in the text channel', async () => {
    const w = await world()
    try {
      const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0xfe])
      await w.host.fs.writeFile(`${HOSTLESS_HOME}/pic.png`, png)
      const read = exited(await w.exec('demi file read pic.png'))
      expect(read.exitCode).toBe(0)
      expect(read.binaryStdout?.data).toEqual(png)
      expect(read.stdout.delta).toContain('<binary stdout: 10 bytes; not kept beyond this view>')
      // Nothing beyond the view is kept: no output files anywhere.
      expect(read.outputDir).toBeUndefined()
      expect(read.stdout.path).toBeUndefined()
    } finally {
      w.dispose()
    }
  })

  test('ephemeral shells start where asked and never leak state; the namespace bounds cwd', async () => {
    const w = await world()
    try {
      await w.host.fs.mkdir('/tmp/work', { recursive: true })
      const one = exited(await w.exec('pwd; X=1; cd /tmp', { ephemeral: true, cwd: '/tmp/work' }))
      expect(one.stdout.delta).toBe('/tmp/work\n')
      const two = exited(await w.exec('pwd; echo "x=$X"'))
      expect(two.stdout.delta).toBe(`${HOSTLESS_HOME}\nx=\n`)
      await expect(w.exec('pwd', { ephemeral: true, cwd: '/etc' })).rejects.toThrow('outside the hostless namespace')
      expect(await w.env.disposeShell(one.shellId)).toBe(true)
    } finally {
      w.dispose()
    }
  })
})

