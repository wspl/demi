import { mkdir, mkdtemp, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import {
  BashEnvironment,
  type CommandIO,
  type CommandStorage,
  type Host,
  type HostDirent,
  type HostFileStat,
  type HostFileSystem,
  type HostProcess,
  type HostStore,
} from '@demicodes/shell'
import { LocalHost } from '@demicodes/host-local'
import { bytesToBase64, encodeUtf8 } from '@demicodes/utils'
import { createCodingCommandRegistry, createDemiCommand } from '../index'

test('demi read returns a text file as text', async () => {
  const { env } = await createDemiEnvironment()
  const created = await env.exec({ script: "demi create note.txt <<'EOF'\nhello world\nEOF" })
  const read = await env.exec({ shellId: created.shellId, script: 'demi read note.txt' })
  expect(read.stdout.delta).toBe('hello world\n')
})

test('demi read emits raw bytes; binary files surface as binaryStdout at the boundary', async () => {
  const { env, host } = await createDemiEnvironment()
  // A real (invalid-UTF-8) binary payload written through Host.fs directly.
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe])
  await host.fs.writeFile('shot.png', png, { cwd: host.defaultCwd })

  const read = await env.exec({ script: 'demi read shot.png' })
  if (read.status !== 'exited') throw new Error('expected exited result')
  expect(read.exitCode).toBe(0)
  expect(read.binaryStdout?.data).toEqual(png)
  expect(read.stdout.delta).toContain(`<binary stdout: ${png.length} bytes; raw bytes at /@/commands/`)

  // Bytes pipe cleanly into downstream commands.
  const counted = await env.exec({ shellId: read.shellId, script: 'demi read shot.png | wc -c' })
  if (counted.status !== 'exited') throw new Error('expected exited result')
  expect(counted.stdout.delta.trim()).toBe(String(png.length))
})

test('demi --help documents byte-stream reads, and any word stays usable as a file name', async () => {
  const { env } = await createDemiEnvironment()
  const help = await env.exec({ script: 'demi --help' })

  expect(help.status).toBe('exited')
  if (help.status !== 'exited') throw new Error('expected exited result')
  expect(help.stdout.delta).toContain('shown to you as viewable media')
  expect(help.stdout.delta).toContain('demi read assets/clip.mp4')

  const leafHelp = await env.exec({ shellId: help.shellId, script: 'demi read --help' })
  expect(leafHelp.status).toBe('exited')
  if (leafHelp.status !== 'exited') throw new Error('expected exited result')
  expect(leafHelp.stdout.delta).toContain('demi read: Read a file.')

  // Help is a flag, so no word is reserved: a file named "prompt" is just a file.
  const created = await env.exec({ shellId: help.shellId, script: "demi create prompt <<'EOF'\nnot help\nEOF" })
  expect(created.status).toBe('exited')
  const read = await env.exec({ shellId: help.shellId, script: 'demi read prompt' })
  expect(read.status).toBe('exited')
  if (read.status !== 'exited') throw new Error('expected exited result')
  expect(read.exitCode).toBe(0)
  expect(read.stdout.delta).toBe('not help\n')
})

test('demi create writes a new file from heredoc content', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create src/foo.txt <<'EOF'\nhello\nEOF",
  })
  expect(created.stdout.delta).toBe('Created src/foo.txt\n')
  expect(fileDiffs(created)[0]).toMatchObject({
    type: 'file_diff',
    action: 'create',
    path: 'src/foo.txt',
    oldPath: null,
    newPath: 'src/foo.txt',
  })
  expect(String(fileDiffs(created)[0].unifiedDiff)).toContain('+++ b/src/foo.txt')

  const read = await env.exec({ shellId: created.shellId, script: 'cat src/foo.txt' })
  expect(read.stdout.delta).toBe('hello\n')
})

test('demi allows paths outside default cwd when Host.fs allows them', async () => {
  const parent = await mkdtemp(join(tmpdir(), 'demi-boundary-'))
  const defaultCwd = join(parent, 'default-cwd')
  await mkdir(defaultCwd)
  const host = new LocalHost(defaultCwd)
  const env = new BashEnvironment({
    host,
    commands: createCodingCommandRegistry({ demiHost: host }),
    shellIdFactory: () => 'demi-boundary-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const absoluteOutside = join(parent, 'absolute-outside.txt')
  const absolute = await env.exec({
    script: `demi create ${JSON.stringify(absoluteOutside)} <<'EOF'\nnope\nEOF`,
  })
  expect(absolute.status).toBe('exited')
  if (absolute.status !== 'exited') throw new Error('expected exited result')
  expect(absolute.exitCode).toBe(0)
  await expect(readFile(absoluteOutside, 'utf8')).resolves.toBe('nope\n')

  const relative = await env.exec({
    shellId: absolute.shellId,
    script: "demi create ../relative-outside.txt <<'EOF'\nnope\nEOF",
  })
  expect(relative.status).toBe('exited')
  if (relative.status !== 'exited') throw new Error('expected exited result')
  expect(relative.exitCode).toBe(0)
  await expect(readFile(join(parent, 'relative-outside.txt'), 'utf8')).resolves.toBe('nope\n')
})

test('demi patch can modify paths outside default cwd when Host.fs allows them', async () => {
  const parent = await mkdtemp(join(tmpdir(), 'demi-patch-boundary-'))
  const root = join(parent, 'default-cwd')
  await mkdir(root)
  const outsidePath = join(parent, 'outside.txt')
  const host = new LocalHost(root)
  const env = new BashEnvironment({
    host,
    commands: createCodingCommandRegistry({ demiHost: host }),
    shellIdFactory: () => 'demi-patch-boundary-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const created = await env.exec({
    script: "demi create inside.txt <<'EOF'\ninside\nEOF",
  })
  const patched = await env.exec({
    shellId: created.shellId,
    script: `demi patch <<'PATCH'\n--- a/inside.txt\n+++ b/inside.txt\n@@ -1 +1 @@\n-inside\n+changed\n--- /dev/null\n+++ ${outsidePath}\n@@ -0,0 +1 @@\n+outside\nPATCH`,
  })

  expect(patched.status).toBe('exited')
  if (patched.status !== 'exited') throw new Error('expected exited result')
  expect(patched.exitCode).toBe(0)

  const inside = await env.exec({ shellId: created.shellId, script: 'cat inside.txt' })
  expect(inside.stdout.delta).toBe('changed\n')
  await expect(readFile(outsidePath, 'utf8')).resolves.toBe('outside\n')
})

test('demi edit replaces exact text and fails on ambiguous matches', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create file.txt <<'EOF'\none\ntwo\ntwo\nEOF",
  })

  const ambiguous = await env.exec({
    shellId: created.shellId,
    script: 'demi edit file.txt --old two --new changed',
  })
  if (ambiguous.status !== 'exited') throw new Error('expected exited result')
  expect(ambiguous.exitCode).toBe(1)
  expect(ambiguous.stderr.delta).toContain('Multiple matches')

  const edited = await env.exec({
    shellId: created.shellId,
    script: 'demi edit file.txt --old two --new changed --occurrence 2',
  })
  expect(edited.stdout.delta).toBe('Edited file.txt\n')
  expect(fileDiffs(edited)[0]).toMatchObject({
    action: 'edit',
    path: 'file.txt',
  })

  const read = await env.exec({ shellId: created.shellId, script: 'cat file.txt' })
  expect(read.stdout.delta).toBe('one\ntwo\nchanged\n')
})

test('demi edit uses context only when it disambiguates to one nearest match', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create context.txt <<'EOF'\ntarget\nmiddle\ntarget\nEOF",
  })

  const ambiguous = await env.exec({
    shellId: created.shellId,
    script: 'demi edit context.txt --old target --new changed --context 2',
  })
  if (ambiguous.status !== 'exited') throw new Error('expected exited result')
  expect(ambiguous.exitCode).toBe(1)
  expect(ambiguous.stderr.delta).toContain('Context line 2 is ambiguous')
  expect(ambiguous.stderr.delta).toContain('occurrence 1 at line 1')
  expect(ambiguous.stderr.delta).toContain('occurrence 2 at line 3')

  const unchanged = await env.exec({ shellId: created.shellId, script: 'cat context.txt' })
  expect(unchanged.stdout.delta).toBe('target\nmiddle\ntarget\n')

  const edited = await env.exec({
    shellId: created.shellId,
    script: 'demi edit context.txt --old target --new changed --context 3',
  })
  expect(edited.stdout.delta).toBe('Edited context.txt\n')

  const read = await env.exec({ shellId: created.shellId, script: 'cat context.txt' })
  expect(read.stdout.delta).toBe('target\nmiddle\nchanged\n')
})

test('demi edit rejects empty old text without modifying the file', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create empty-old.txt <<'EOF'\ncontent\nEOF",
  })

  const failed = await env.exec({
    shellId: created.shellId,
    script: 'demi edit empty-old.txt --old "" --new changed',
  })
  expect(failed.status).toBe('exited')
  if (failed.status !== 'exited') throw new Error('expected exited result')
  expect(failed.exitCode).toBe(1)
  expect(failed.stderr.delta).toContain('Old text must not be empty')

  const unchanged = await env.exec({ shellId: created.shellId, script: 'cat empty-old.txt' })
  expect(unchanged.stdout.delta).toBe('content\n')
})

test('demi patch applies a unified diff', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create patch.txt <<'EOF'\none\ntwo\nEOF",
  })

  const patched = await env.exec({
    shellId: created.shellId,
    script: "demi patch <<'PATCH'\n--- a/patch.txt\n+++ b/patch.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\nPATCH",
  })
  expect(patched.stdout.delta).toBe('Patched 1 file(s)\n')
  expect(fileDiffs(patched)[0]).toMatchObject({
    action: 'patch',
    path: 'patch.txt',
  })

  const read = await env.exec({ shellId: created.shellId, script: 'cat patch.txt' })
  expect(read.stdout.delta).toBe('one\nthree\n')
})

test('demi patch accepts unified diff headers with timestamps', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create timed.txt <<'EOF'\nold\nEOF",
  })

  const patched = await env.exec({
    shellId: created.shellId,
    script:
      "demi patch <<'PATCH'\n--- a/timed.txt 2026-06-17 00:00:00.000000000 +0800\n+++ b/timed.txt 2026-06-17 00:00:01.000000000 +0800\n@@ -1 +1 @@\n-old\n+new\nPATCH",
  })
  expect(patched.stdout.delta).toBe('Patched 1 file(s)\n')

  const read = await env.exec({ shellId: created.shellId, script: 'cat timed.txt' })
  expect(read.stdout.delta).toBe('new\n')
})

test('demi patch applies multiple files and creates new files', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create existing.txt <<'EOF'\none\nEOF",
  })

  const patched = await env.exec({
    shellId: created.shellId,
    script:
      "demi patch <<'PATCH'\n--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1 @@\n-one\n+changed\n--- /dev/null\n+++ b/nested/new.txt\n@@ -0,0 +1,2 @@\n+new\n+file\nPATCH",
  })
  expect(patched.stdout.delta).toBe('Patched 2 file(s)\n')
  expect(fileDiffs(patched)).toHaveLength(2)
  expect(fileDiffs(patched)[1]).toMatchObject({
    action: 'patch',
    path: 'nested/new.txt',
    oldPath: null,
    newPath: 'nested/new.txt',
  })

  const existing = await env.exec({ shellId: created.shellId, script: 'cat existing.txt' })
  expect(existing.stdout.delta).toBe('changed\n')
  const added = await env.exec({ shellId: created.shellId, script: 'cat nested/new.txt' })
  expect(added.stdout.delta).toBe('new\nfile\n')
})

test('demi patch deletes files with a /dev/null target', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create doomed.txt <<'EOF'\nremove\nEOF",
  })

  const patched = await env.exec({
    shellId: created.shellId,
    script: "demi patch <<'PATCH'\n--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-remove\nPATCH",
  })
  expect(patched.stdout.delta).toBe('Patched 1 file(s)\n')
  expect(fileDiffs(patched)[0]).toMatchObject({
    action: 'delete',
    path: 'doomed.txt',
    oldPath: 'doomed.txt',
    newPath: null,
  })

  const missing = await env.exec({ shellId: created.shellId, script: 'test ! -e doomed.txt' })
  expect(missing.status).toBe('exited')
  if (missing.status !== 'exited') throw new Error('expected exited result')
  expect(missing.exitCode).toBe(0)
})

test('demi patch validates all files before writing any changes', async () => {
  const { env } = await createDemiEnvironment()

  const created = await env.exec({
    script: "demi create first.txt <<'EOF'\nfirst\nEOF\ndemi create second.txt <<'EOF'\nsecond\nEOF",
  })

  const failed = await env.exec({
    shellId: created.shellId,
    script:
      "demi patch <<'PATCH'\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-wrong\n+changed\nPATCH",
  })
  expect(failed.status).toBe('exited')
  if (failed.status !== 'exited') throw new Error('expected exited result')
  expect(failed.exitCode).toBe(1)
  expect(failed.stderr.delta).toContain('Patch does not apply to second.txt')

  const first = await env.exec({ shellId: created.shellId, script: 'cat first.txt' })
  expect(first.stdout.delta).toBe('first\n')
})

test('demi patch rolls back files when a later write fails', async () => {
  const host = new FailingWriteHost('/workspace', {
    'first.txt': 'first\n',
    'second.txt': 'second\n',
  })
  const command = createDemiCommand(host)
  const patch = command.subcommands?.find((subcommand) => subcommand.name === 'patch')
  if (!patch?.run) throw new Error('missing demi patch command')
  const output = commandOutput()

  const result = await patch.run({
    argv: ['demi', 'patch'],
    parsed: {
      path: ['demi', 'patch'],
      help: false,
      json: false,
      values: {
        patch:
          '--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-second\n+changed\n',
      },
    },
    stdin: { text: '', bytes: new Uint8Array(0) },
    env: {},
    cwd: '/workspace',
    io: output.io,
    storage: noopStorage,
  })

  expect(result.exitCode).toBe(1)
  expect(output.stderr()).toContain('simulated write failure')
  expect(host.read('first.txt')).toBe('first\n')
  expect(host.read('second.txt')).toBe('second\n')
})

async function createDemiEnvironment(): Promise<{ env: BashEnvironment; host: LocalHost }> {
  const root = await mkdtemp(join(tmpdir(), 'demi-scratch-'))
  const host = new LocalHost(root)
  const env = new BashEnvironment({
    host,
    commands: createCodingCommandRegistry({ demiHost: host }),
    shellIdFactory: () => 'demi-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  return { env, host }
}

function fileDiffs(result: { status: string; commandMetadata?: Array<{ metadata: unknown }> }): Record<string, unknown>[] {
  if (result.status !== 'exited') throw new Error('expected exited result')
  const metadata = result.commandMetadata?.[0]?.metadata
  if (!isRecord(metadata) || metadata.type !== 'file_diffs' || !Array.isArray(metadata.diffs)) {
    throw new Error('missing demi file diff metadata')
  }
  return metadata.diffs.map((diff) => {
    if (!isRecord(diff)) throw new Error('invalid demi diff metadata')
    return diff
  })
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

const noopStorage: CommandStorage = {
  readJson: async () => null,
  writeJson: async () => {},
  delete: async () => {},
  list: async () => [],
}

function commandOutput(): { io: CommandIO; stdout: () => string; stderr: () => string } {
  const stdout: string[] = []
  const stderr: string[] = []
  return {
    io: {
      stdout: async (data) => {
        stdout.push(text(data))
      },
      stderr: async (data) => {
        stderr.push(text(data))
      },
    },
    stdout: () => stdout.join(''),
    stderr: () => stderr.join(''),
  }
}

class FailingWriteHost implements Host {
  readonly defaultCwd: string
  readonly fs: FailingWriteFileSystem
  readonly store: HostStore = new MemoryHostStore()
  readonly process: HostProcess = {
    spawn: async (): Promise<never> => {
      throw new Error('Host.process.spawn must not be used by demi file operations')
    },
  }

  constructor(
    defaultCwd: string,
    files: Record<string, string>,
  ) {
    this.defaultCwd = defaultCwd
    this.fs = new FailingWriteFileSystem(defaultCwd, files)
  }

  read(path: string): string | undefined {
    return this.fs.readText(path)
  }

}

class MemoryHostStore implements HostStore {
  async readJson<T>(): Promise<T | null> { return null }
  async writeJson<T>(): Promise<void> {}
  async delete(): Promise<void> {}
  async list(): Promise<string[]> { return [] }
}

class FailingWriteFileSystem implements HostFileSystem {
  private readonly files = new Map<string, string>()

  constructor(
    private readonly root: string,
    files: Record<string, string>,
  ) {
    for (const [path, content] of Object.entries(files)) {
      this.files.set(this.resolve(path), content)
    }
  }

  readText(path: string): string | undefined {
    return this.files.get(this.resolve(path))
  }

  async readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array> {
    const content = this.files.get(this.resolve(path, options?.cwd))
    if (content === undefined) throw new Error(`ENOENT: ${path}`)
    return new TextEncoder().encode(content)
  }

  async writeFile(path: string, data: Uint8Array, options?: { cwd?: string }): Promise<void> {
    const target = this.resolve(path, options?.cwd)
    const content = text(data)
    this.files.set(target, content)
    if (target === `${this.root}/second.txt` && content === 'changed\n') {
      throw new Error('simulated write failure')
    }
  }

  async appendFile(path: string, data: Uint8Array, options?: { cwd?: string }): Promise<void> {
    const target = this.resolve(path, options?.cwd)
    this.files.set(target, `${this.files.get(target) ?? ''}${text(data)}`)
  }

  async exists(path: string, options?: { cwd?: string }): Promise<boolean> {
    return this.files.has(this.resolve(path, options?.cwd))
  }

  async stat(path: string, options?: { cwd?: string }): Promise<HostFileStat> {
    const content = this.files.get(this.resolve(path, options?.cwd))
    if (content === undefined) throw new Error(`ENOENT: ${path}`)
    return {
      isFile: true,
      isDirectory: false,
      isSymbolicLink: false,
      mode: 0o644,
      size: content.length,
      mtime: new Date(0),
    }
  }

  async lstat(path: string, options?: { cwd?: string }): Promise<HostFileStat> {
    return this.stat(path, options)
  }

  async readdir(path: string, options: { cwd?: string; withFileTypes: true }): Promise<HostDirent[]>
  async readdir(path: string, options?: { cwd?: string; withFileTypes?: false }): Promise<string[]>
  async readdir(): Promise<string[] | HostDirent[]> { return [] }
  async mkdir(): Promise<void> {}
  async rm(path: string, options?: { cwd?: string }): Promise<void> { this.files.delete(this.resolve(path, options?.cwd)) }
  async cp(): Promise<void> { throw new Error('not implemented') }
  async mv(): Promise<void> { throw new Error('not implemented') }
  async chmod(): Promise<void> {}
  async symlink(): Promise<void> { throw new Error('not implemented') }
  async link(): Promise<void> { throw new Error('not implemented') }
  async readlink(): Promise<string> { throw new Error('not implemented') }
  async realpath(path: string, options?: { cwd?: string }): Promise<string> { return this.resolve(path, options?.cwd) }
  async utimes(): Promise<void> {}

  private resolve(path: string, cwd?: string): string {
    if (path.startsWith('/')) return path
    return `${cwd ?? this.root}/${path}`
  }
}

function text(data: string | Uint8Array): string {
  return typeof data === 'string' ? data : new TextDecoder().decode(data)
}
