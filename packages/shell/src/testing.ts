// Test helpers for packages exercising the Host contract. Shipped as the
// `@demicodes/shell/testing` entrypoint, never imported by runtime code:
// the in-memory store and the conformance suite, runtime-neutral so the
// suite runs on tinyjs too. The Node Host tests run it against is
// `LocalHost` under `@demicodes/host-virtual/testing`.
import type { Host, HostSpawnHandle, HostStore } from './host'
import { collectBytes, decodeUtf8, encodeUtf8, errorCode } from '@demicodes/utils'

/** In-memory `HostStore` for tests (values held by reference, no cloning). */
export function memoryHostStore(): HostStore {
  const map = new Map<string, unknown>()
  return {
    readJson: async <T>(key: string) => (map.has(key) ? (map.get(key) as T) : null),
    writeJson: async (key, value) => {
      map.set(key, value)
    },
    delete: async (key) => {
      map.delete(key)
    },
    list: async (prefix) => [...map.keys()].filter((key) => key.startsWith(prefix)),
  }
}

/**
 * The Host conformance suite: what every `Host` implementation must do,
 * as cases any harness can run — `bun:test` for a Host on Bun, the tinyjs
 * harness for the runner's machine layer. The suite is runtime-neutral: it
 * imports nothing but the contract and `@demicodes/utils`.
 *
 * The process cases spawn `sh`, `printf`, `sleep`, `printenv`, `cat` and
 * `/bin/echo` from `path`.
 */
export interface HostConformanceOptions {
  host: Host
  /** A fresh, writable, absolute directory on the host; every case works under it. */
  root: string
  /** `PATH` for the spawned programs (default `/usr/bin:/bin`). */
  path?: string
}

export interface HostConformanceCase {
  name: string
  run(): Promise<void>
}

export function hostConformanceCases(options: HostConformanceOptions): HostConformanceCase[] {
  const { host, root } = options
  const env = { PATH: options.path ?? '/usr/bin:/bin' }
  const fs = host.fs
  // Process cases apply only to a Host that runs processes.
  const spawn = host.process.spawn?.bind(host.process)
  const processCase = (name: string, run: (spawn: NonNullable<Host['process']['spawn']>) => Promise<void>): HostConformanceCase[] =>
    spawn ? [{ name, run: () => run(spawn) }] : []
  const text = (value: string) => encodeUtf8(value)
  const read = async (path: string) => decodeUtf8(await fs.readFile(path))
  const codeOf = async (action: () => Promise<unknown>): Promise<string | null> => {
    try {
      await action()
    } catch (error) {
      return errorCode(error)
    }
    return null
  }

  return [
    {
      name: 'host: defaultCwd is an absolute path; identity is numeric ids and a hostname',
      run: async () => {
        ok(host.defaultCwd.startsWith('/'), `defaultCwd is absolute: ${host.defaultCwd}`)
        ok(Number.isInteger(host.identity.uid) && Number.isInteger(host.identity.gid), 'identity uid/gid are integers')
        equal(typeof host.identity.hostname, 'string', 'identity hostname')
      },
    },
    ...processCase('process: spawn captures stdout and the exit code', async (spawn) => {        const result = await outputOf(await spawn({ command: 'printf', args: ['hello\\n'], env }))
        equal(result.stdout, 'hello\n', 'stdout')
        equal(result.exit.exitCode, 0, 'exit code')
        equal(result.exit.spawnError, undefined, 'no spawn error')
    }),
    ...processCase('process: stdout and stderr are separate; a non-zero exit is reported', async (spawn) => {        const result = await outputOf(await spawn({ command: 'sh', args: ['-c', 'echo out; echo err >&2; exit 3'], env }))
        equal(result.stdout, 'out\n', 'stdout')
        equal(result.stderr, 'err\n', 'stderr')
        equal(result.exit.exitCode, 3, 'exit code')
    }),
    ...processCase('process: stdin reaches the child and ends when closed', async (spawn) => {        const handle = await spawn({ command: 'sh', args: ['-c', 'IFS= read -r line; printf "%s" "$line"'], env })
        await handle.writeStdin(text('from stdin\n'))
        await handle.closeStdin()
        const result = await outputOf(handle)
        equal(result.stdout, 'from stdin', 'stdout')
        equal(result.exit.exitCode, 0, 'exit code')
    }),
    ...processCase('process: kill ends a foreground process with SIGTERM', async (spawn) => {        const handle = await spawn({ command: 'sleep', args: ['10'], env })
        await handle.kill()
        const result = await outputOf(handle)
        equal(result.exit.exitCode, null, 'exit code')
        equal(result.exit.signal, 'SIGTERM', 'signal')
    }),
    ...processCase('process: children receive only the env passed to spawn', async (spawn) => {        const result = await outputOf(await spawn({ command: 'printenv', args: ['HOME'], env }))
        equal(result.exit.exitCode, 1, 'printenv exits 1 for an unset name')
        equal(result.stdout, '', 'stdout')
    }),
    ...processCase('process: cwd is honoured', async (spawn) => {        const dir = `${root}/cwd-honoured`
        await fs.mkdir(dir, { recursive: true })
        const result = await outputOf(await spawn({ command: 'sh', args: ['-c', 'pwd'], cwd: dir, env }))
        equal(result.exit.exitCode, 0, 'exit code')
        ok(result.stdout.trimEnd().endsWith('/cwd-honoured'), `pwd inside the cwd: ${result.stdout}`)
    }),
    ...processCase('process: a missing binary is executable_not_found', async (spawn) => {        const result = await outputOf(await spawn({ command: 'definitely-not-a-host-binary', env }))
        equal(result.exit.exitCode, null, 'exit code')
        equal(result.exit.spawnError?.kind, 'executable_not_found', 'spawn error kind')
    }),
    ...processCase('process: a missing cwd is cwd_unusable, not a missing binary', async (spawn) => {        const result = await outputOf(await spawn({ command: '/bin/echo', args: ['ok'], cwd: `${root}/never-created`, env }))
        equal(result.exit.exitCode, null, 'exit code')
        equal(result.exit.spawnError?.kind, 'cwd_unusable', 'spawn error kind')
    }),
    ...processCase('process: a missing binary with a live cwd is executable_not_found', async (spawn) => {        const dir = `${root}/live-cwd`
        await fs.mkdir(dir, { recursive: true })
        const cwd = await host.process.openCwd(dir)
        try {
          const result = await outputOf(await spawn({ command: 'definitely-not-a-host-binary', cwd: cwd.spawnPath(), env }))
          equal(result.exit.spawnError?.kind, 'executable_not_found', 'spawn error kind')
        } finally {
          await cwd.close()
        }
    }),
    ...processCase('process: openCwd walks directories, refuses missing ones, snapshots and restores', async (spawn) => {        const dir = `${root}/open-cwd`
        await fs.mkdir(`${dir}/sub`, { recursive: true })
        const cwd = await host.process.openCwd(dir)
        try {
          await cwd.chdir('sub')
          ok(cwd.path.endsWith('/open-cwd/sub'), `relative chdir: ${cwd.path}`)
          await cwd.chdir('..')
          ok(cwd.path.endsWith('/open-cwd'), `chdir ..: ${cwd.path}`)
          const snapshot = await cwd.snapshot()
          await cwd.chdir(`${dir}/sub`)
          ok(cwd.path.endsWith('/open-cwd/sub'), `absolute chdir: ${cwd.path}`)
          snapshot.restore()
          ok(cwd.path.endsWith('/open-cwd'), `restored: ${cwd.path}`)
          equal(await codeOf(() => cwd.chdir('missing')), 'ENOENT', 'chdir into a missing directory')
        } finally {
          await cwd.close()
        }
        equal(await codeOf(() => host.process.openCwd(`${root}/open-cwd-missing`)), 'ENOENT', 'openCwd on a missing directory')
    }),
    {
      name: 'fs: write, append, read, readdir, stat, exists, rm',
      run: async () => {
        const dir = `${root}/basic`
        await fs.mkdir(`${dir}/src`, { recursive: true })
        await fs.writeFile('src/file.txt', text('hello\n'), { cwd: dir })
        await fs.appendFile('src/file.txt', text('tail\n'), { cwd: dir })
        equal(await read(`${dir}/src/file.txt`), 'hello\ntail\n', 'content')
        equal(await fs.readdir('src', { cwd: dir }), ['file.txt'], 'readdir names')
        const typed = await fs.readdir('src', { cwd: dir, withFileTypes: true })
        equal(typed.map((entry) => ({ name: entry.name, isFile: entry.isFile, isDirectory: entry.isDirectory })), [{ name: 'file.txt', isFile: true, isDirectory: false }], 'readdir dirents')
        const stat = await fs.stat('src/file.txt', { cwd: dir })
        equal([stat.isFile, stat.isDirectory, stat.isSymbolicLink, stat.size], [true, false, false, 'hello\ntail\n'.length], 'stat')
        ok(stat.mtime instanceof Date && !Number.isNaN(stat.mtime.getTime()), 'stat mtime is a Date')
        equal((await fs.stat('src', { cwd: dir })).isDirectory, true, 'stat of a directory')
        await fs.rm('src/file.txt', { cwd: dir, force: true })
        equal(await fs.exists('src/file.txt', { cwd: dir }), false, 'exists after rm')
        equal(await fs.exists('src', { cwd: dir }), true, 'exists for a directory')
      },
    },
    {
      name: 'fs: createParents, recursive cp, mv, recursive rm, force',
      run: async () => {
        const dir = `${root}/tree`
        await fs.mkdir(`${dir}/a/b`, { recursive: true })
        await fs.writeFile(`${dir}/a/b/f.txt`, text('x'))
        await fs.writeFile(`${dir}/a/new/dir/g.txt`, text('y'), { createParents: true })
        equal(await read(`${dir}/a/new/dir/g.txt`), 'y', 'createParents')
        await fs.cp(`${dir}/a`, `${dir}/c`, { recursive: true })
        equal(await read(`${dir}/c/b/f.txt`), 'x', 'cp -r copied the tree')
        equal(await read(`${dir}/a/b/f.txt`), 'x', 'cp -r kept the source')
        await fs.mv(`${dir}/c`, `${dir}/d`)
        equal(await fs.exists(`${dir}/c`), false, 'mv removed the source')
        equal(await read(`${dir}/d/b/f.txt`), 'x', 'mv moved the tree')
        await fs.rm(`${dir}/d`, { recursive: true })
        equal(await fs.exists(`${dir}/d`), false, 'rm -r')
        await fs.rm(`${dir}/absent`, { force: true })
        equal(await codeOf(() => fs.rm(`${dir}/absent`)), 'ENOENT', 'rm without force on a missing path')
      },
    },
    {
      name: 'fs: symlink, readlink, lstat, realpath, link, chmod, utimes',
      run: async () => {
        const dir = `${root}/links`
        await fs.mkdir(dir, { recursive: true })
        await fs.writeFile(`${dir}/target.txt`, text('t'))
        await fs.symlink('target.txt', `${dir}/link`)
        equal(await fs.readlink(`${dir}/link`), 'target.txt', 'readlink')
        equal((await fs.lstat(`${dir}/link`)).isSymbolicLink, true, 'lstat sees the link')
        equal((await fs.stat(`${dir}/link`)).isFile, true, 'stat follows the link')
        ok((await fs.realpath(`${dir}/link`)).endsWith('/links/target.txt'), 'realpath resolves the link')
        const entries = await fs.readdir(dir, { withFileTypes: true })
        equal(entries.find((entry) => entry.name === 'link')?.isSymbolicLink, true, 'readdir marks the link')
        await fs.link(`${dir}/target.txt`, `${dir}/hard`)
        equal(await read(`${dir}/hard`), 't', 'hard link content')
        await fs.chmod(`${dir}/target.txt`, 0o600)
        equal((await fs.stat(`${dir}/target.txt`)).mode & 0o777, 0o600, 'chmod')
        const when = new Date(1_600_000_000_000)
        await fs.utimes(`${dir}/target.txt`, when, when)
        equal((await fs.stat(`${dir}/target.txt`)).mtime.getTime(), when.getTime(), 'utimes')
      },
    },
    {
      name: 'fs: errors carry errno codes',
      run: async () => {
        const dir = `${root}/errors`
        await fs.mkdir(dir, { recursive: true })
        equal(await codeOf(() => fs.readFile(`${dir}/missing`)), 'ENOENT', 'readFile of a missing file')
        equal(await codeOf(() => fs.stat(`${dir}/missing`)), 'ENOENT', 'stat of a missing file')
        equal(await codeOf(() => fs.mkdir(dir)), 'EEXIST', 'mkdir of an existing directory')
        equal(await codeOf(() => fs.readdir(`${dir}/missing`)), 'ENOENT', 'readdir of a missing directory')
      },
    },
    ...processCase('fs and process share one namespace: a file written through fs is read by a process', async (spawn) => {        const dir = `${root}/shared`
        await fs.mkdir(dir, { recursive: true })
        await fs.writeFile(`${dir}/seen.txt`, text('seen by cat\n'))
        const result = await outputOf(await spawn({ command: 'cat', args: ['seen.txt'], cwd: dir, env }))
        equal(result.stdout, 'seen by cat\n', 'cat output')
        const wrote = await outputOf(await spawn({ command: 'sh', args: ['-c', 'printf back > written.txt'], cwd: dir, env }))
        equal(wrote.exit.exitCode, 0, 'shell wrote')
        equal(await read(`${dir}/written.txt`), 'back', 'read through fs')
    }),
    {
      name: 'store: JSON round trip with bytes and bigints, list by prefix, delete',
      run: async () => {
        const store = host.store
        await store.writeJson('conformance/a', { big: 10n, bytes: new Uint8Array([1, 2]) })
        await store.writeJson('conformance/b', [1, 2, 3])
        const a = await store.readJson<{ big: bigint; bytes: Uint8Array }>('conformance/a')
        equal(typeof a?.big, 'bigint', 'bigint survives')
        ok(a?.bytes instanceof Uint8Array, 'Uint8Array survives')
        equal(Array.from(a?.bytes ?? []), [1, 2], 'bytes content')
        equal(await store.readJson('conformance/b'), [1, 2, 3], 'array round trip')
        equal(await store.list('conformance'), ['conformance/a', 'conformance/b'], 'list by prefix')
        await store.delete('conformance/a')
        equal(await store.readJson('conformance/a'), null, 'deleted key reads null')
        equal(await store.list('conformance'), ['conformance/b'], 'list after delete')
        await store.delete('conformance/b')
      },
    },
  ]
}

async function outputOf(handle: HostSpawnHandle) {
  const [stdout, stderr, exit] = await Promise.all([collectBytes(handle.stdout), collectBytes(handle.stderr), handle.wait()])
  return { stdout: decodeUtf8(stdout), stderr: decodeUtf8(stderr), exit }
}

function equal(actual: unknown, expected: unknown, what: string): void {
  const a = stringify(actual)
  const e = stringify(expected)
  if (a !== e) throw new Error(`${what}: expected ${e}, got ${a}`)
}

function ok(condition: boolean, what: string): void {
  if (!condition) throw new Error(what)
}

function stringify(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) => (typeof item === 'bigint' ? `${item}n` : item)) ?? 'undefined'
}
