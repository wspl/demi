import { expect, test } from 'bun:test'
import { mkdir, mkdtemp, realpath, symlink } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createDemiCommand } from '@demicodes/coding-agent'
import { buildManifest, writeManifestDirectory } from '@demicodes/command-loader'
import { LocalHost } from '@demicodes/host-local'
import { bundleForTinyjs, tinyjsBinary } from '@demicodes/host-runner/testing'

// tinyjs in command mode: the bundle packed by tinyjsc, reached through a
// symlink named after the root, running `demi file` runtime modules from a
// manifest directory against the real filesystem.
test('command mode runs demi file runtime commands on tinyjs', async () => {
  const work = await realpath(await mkdtemp(join(tmpdir(), 'demi-command-mode-')))
  const bundle = join(work, 'entry.mjs')
  await bundleForTinyjs(join(import.meta.dir, '..', 'tinyjs', 'entry.ts'), bundle)
  const packed = join(work, 'demi-cli')
  const pack = Bun.spawnSync([tinyjsBinary('tinyjsc'), bundle, '--bin', tinyjsBinary(), '--out', packed], { stdout: 'pipe', stderr: 'pipe' })
  expect(pack.exitCode, pack.stderr.toString()).toBe(0)
  const bin = join(work, 'bin')
  await mkdir(bin)
  await symlink(packed, join(bin, 'demi'))
  await symlink(packed, join(bin, 'nope'))
  await symlink(packed, join(bin, 'demi-runner'))

  const transpiler = new Bun.Transpiler({ loader: 'ts', target: 'browser' })
  const manifest = await buildManifest([createDemiCommand()], { transpile: (source) => transpiler.transformSync(source) })
  const commands = join(work, 'commands', manifest.hash)
  await writeManifestDirectory(manifest, commands, new LocalHost(work, { storeRoot: join(work, 'store') }).fs)

  const project = join(work, 'project')
  await mkdir(project)
  const run = (name: string, args: string[], stdin = '') => {
    const started = performance.now()
    const result = Bun.spawnSync([join(bin, name), ...args], {
      cwd: project,
      env: { PATH: process.env.PATH ?? '/usr/bin:/bin', HOME: work, DEMI_COMMANDS_DIR: commands },
      stdin: new TextEncoder().encode(stdin),
      stdout: 'pipe',
      stderr: 'pipe',
    })
    return { code: result.exitCode, stdout: result.stdout.toString(), stderr: result.stderr.toString(), ms: performance.now() - started }
  }

  const created = run('demi', ['file', 'create', 'notes.md'], 'alpha\nbeta\n')
  expect(created.code, created.stderr).toBe(0)
  expect(created.stdout).toBe('Created notes.md\n')
  expect(await Bun.file(join(project, 'notes.md')).text()).toBe('alpha\nbeta\n')

  const read = run('demi', ['file', 'read', 'notes.md'])
  expect(read.code, read.stderr).toBe(0)
  expect(read.stdout).toBe('alpha\nbeta\n')
  console.log(`command mode: demi file read in ${read.ms.toFixed(0)}ms`)

  const help = run('demi', ['file', '--help'])
  expect(help.code).toBe(0)
  expect(help.stdout).toContain('demi file read')

  // An rpc leaf has no transport in a standalone command-mode process.
  const todo = run('demi', ['todo', 'list'])
  expect(todo.code).toBe(1)
  expect(todo.stderr).toContain('rpc command')

  expect(run('nope', ['x']).code).toBe(127)
  const runner = run('demi-runner', [])
  expect(runner.code).toBe(2)
  expect(runner.stderr).toContain('runner mode')
}, 180_000)
