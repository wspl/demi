import { access, mkdir, mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import {
  BashEnvironment,
  type CommandIO,
  type CommandStorage,
  type Host,
  type HostSpawnHandle,
  type HostSpawnParams,
} from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
import { createCodingCommandRegistry, createEditorCommand } from '../index'

test('editor create writes a new file from heredoc content', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create src/foo.txt <<'EOF'\nhello\nEOF",
  })
  expect(created.output.stdoutDelta).toBe('Created src/foo.txt\n')
  expect(editorDiffs(created)[0]).toMatchObject({
    type: 'file_diff',
    action: 'create',
    path: 'src/foo.txt',
    oldPath: null,
    newPath: 'src/foo.txt',
    oldText: '',
    newText: 'hello\n',
  })
  expect(String(editorDiffs(created)[0].unifiedDiff)).toContain('+++ b/src/foo.txt')

  const read = await env.exec({ shellId: created.shellId, script: 'cat src/foo.txt' })
  expect(read.output.stdoutDelta).toBe('hello\n')
})

test('editor rejects paths outside the workspace root', async () => {
  const parent = await mkdtemp(join(tmpdir(), 'demi-editor-boundary-'))
  const root = join(parent, 'workspace')
  await mkdir(root)
  const host = new LocalHost(root)
  const env = new BashEnvironment({
    host,
    commands: createCodingCommandRegistry({ editorHost: host }),
    shellIdFactory: () => 'editor-boundary-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const absoluteOutside = join(parent, 'absolute-outside.txt')
  const absolute = await env.exec({
    script: `editor create ${JSON.stringify(absoluteOutside)} <<'EOF'\nnope\nEOF`,
  })
  expect(absolute.status).toBe('exited')
  if (absolute.status !== 'exited') throw new Error('expected exited result')
  expect(absolute.exitCode).toBe(1)
  expect(absolute.output.stderrDelta).toContain('Path escapes workspace')
  await expect(access(absoluteOutside)).rejects.toThrow()

  const relative = await env.exec({
    shellId: absolute.shellId,
    script: "editor create ../relative-outside.txt <<'EOF'\nnope\nEOF",
  })
  expect(relative.status).toBe('exited')
  if (relative.status !== 'exited') throw new Error('expected exited result')
  expect(relative.exitCode).toBe(1)
  expect(relative.output.stderrDelta).toContain('Path escapes workspace')
  await expect(access(join(parent, 'relative-outside.txt'))).rejects.toThrow()
})

test('editor patch rejects escaped paths before modifying any files', async () => {
  const parent = await mkdtemp(join(tmpdir(), 'demi-editor-patch-boundary-'))
  const root = join(parent, 'workspace')
  await mkdir(root)
  const outsidePath = join(parent, 'outside.txt')
  const host = new LocalHost(root)
  const env = new BashEnvironment({
    host,
    commands: createCodingCommandRegistry({ editorHost: host }),
    shellIdFactory: () => 'editor-patch-boundary-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const created = await env.exec({
    script: "editor create inside.txt <<'EOF'\ninside\nEOF",
  })
  const failed = await env.exec({
    shellId: created.shellId,
    script: `editor patch <<'PATCH'\n--- a/inside.txt\n+++ b/inside.txt\n@@ -1 +1 @@\n-inside\n+changed\n--- /dev/null\n+++ ${outsidePath}\n@@ -0,0 +1 @@\n+outside\nPATCH`,
  })

  expect(failed.status).toBe('exited')
  if (failed.status !== 'exited') throw new Error('expected exited result')
  expect(failed.exitCode).toBe(1)
  expect(failed.output.stderrDelta).toContain('Path escapes workspace')

  const inside = await env.exec({ shellId: created.shellId, script: 'cat inside.txt' })
  expect(inside.output.stdoutDelta).toBe('inside\n')
  await expect(access(outsidePath)).rejects.toThrow()
})

test('editor edit replaces exact text and fails on ambiguous matches', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create file.txt <<'EOF'\none\ntwo\ntwo\nEOF",
  })

  const ambiguous = await env.exec({
    shellId: created.shellId,
    script: 'editor edit file.txt --old two --new changed',
  })
  if (ambiguous.status !== 'exited') throw new Error('expected exited result')
  expect(ambiguous.exitCode).toBe(1)
  expect(ambiguous.output.stderrDelta).toContain('Multiple matches')

  const edited = await env.exec({
    shellId: created.shellId,
    script: 'editor edit file.txt --old two --new changed --occurrence 2',
  })
  expect(edited.output.stdoutDelta).toBe('Edited file.txt\n')
  expect(editorDiffs(edited)[0]).toMatchObject({
    action: 'edit',
    path: 'file.txt',
    oldText: 'one\ntwo\ntwo\n',
    newText: 'one\ntwo\nchanged\n',
  })

  const read = await env.exec({ shellId: created.shellId, script: 'cat file.txt' })
  expect(read.output.stdoutDelta).toBe('one\ntwo\nchanged\n')
})

test('editor edit uses context only when it disambiguates to one nearest match', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create context.txt <<'EOF'\ntarget\nmiddle\ntarget\nEOF",
  })

  const ambiguous = await env.exec({
    shellId: created.shellId,
    script: 'editor edit context.txt --old target --new changed --context 2',
  })
  if (ambiguous.status !== 'exited') throw new Error('expected exited result')
  expect(ambiguous.exitCode).toBe(1)
  expect(ambiguous.output.stderrDelta).toContain('Context line 2 is ambiguous')
  expect(ambiguous.output.stderrDelta).toContain('occurrence 1 at line 1')
  expect(ambiguous.output.stderrDelta).toContain('occurrence 2 at line 3')

  const unchanged = await env.exec({ shellId: created.shellId, script: 'cat context.txt' })
  expect(unchanged.output.stdoutDelta).toBe('target\nmiddle\ntarget\n')

  const edited = await env.exec({
    shellId: created.shellId,
    script: 'editor edit context.txt --old target --new changed --context 3',
  })
  expect(edited.output.stdoutDelta).toBe('Edited context.txt\n')

  const read = await env.exec({ shellId: created.shellId, script: 'cat context.txt' })
  expect(read.output.stdoutDelta).toBe('target\nmiddle\nchanged\n')
})

test('editor edit rejects empty old text without modifying the file', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create empty-old.txt <<'EOF'\ncontent\nEOF",
  })

  const failed = await env.exec({
    shellId: created.shellId,
    script: 'editor edit empty-old.txt --old "" --new changed',
  })
  expect(failed.status).toBe('exited')
  if (failed.status !== 'exited') throw new Error('expected exited result')
  expect(failed.exitCode).toBe(1)
  expect(failed.output.stderrDelta).toContain('Old text must not be empty')

  const unchanged = await env.exec({ shellId: created.shellId, script: 'cat empty-old.txt' })
  expect(unchanged.output.stdoutDelta).toBe('content\n')
})

test('editor patch applies a unified diff', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create patch.txt <<'EOF'\none\ntwo\nEOF",
  })

  const patched = await env.exec({
    shellId: created.shellId,
    script: "editor patch <<'PATCH'\n--- a/patch.txt\n+++ b/patch.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\nPATCH",
  })
  expect(patched.output.stdoutDelta).toBe('Patched 1 file(s)\n')
  expect(editorDiffs(patched)[0]).toMatchObject({
    action: 'patch',
    path: 'patch.txt',
    oldText: 'one\ntwo\n',
    newText: 'one\nthree\n',
  })

  const read = await env.exec({ shellId: created.shellId, script: 'cat patch.txt' })
  expect(read.output.stdoutDelta).toBe('one\nthree\n')
})

test('editor patch accepts unified diff headers with timestamps', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create timed.txt <<'EOF'\nold\nEOF",
  })

  const patched = await env.exec({
    shellId: created.shellId,
    script:
      "editor patch <<'PATCH'\n--- a/timed.txt 2026-06-17 00:00:00.000000000 +0800\n+++ b/timed.txt 2026-06-17 00:00:01.000000000 +0800\n@@ -1 +1 @@\n-old\n+new\nPATCH",
  })
  expect(patched.output.stdoutDelta).toBe('Patched 1 file(s)\n')

  const read = await env.exec({ shellId: created.shellId, script: 'cat timed.txt' })
  expect(read.output.stdoutDelta).toBe('new\n')
})

test('editor patch applies multiple files and creates new files', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create existing.txt <<'EOF'\none\nEOF",
  })

  const patched = await env.exec({
    shellId: created.shellId,
    script:
      "editor patch <<'PATCH'\n--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1 @@\n-one\n+changed\n--- /dev/null\n+++ b/nested/new.txt\n@@ -0,0 +1,2 @@\n+new\n+file\nPATCH",
  })
  expect(patched.output.stdoutDelta).toBe('Patched 2 file(s)\n')
  expect(editorDiffs(patched)).toHaveLength(2)
  expect(editorDiffs(patched)[1]).toMatchObject({
    action: 'patch',
    path: 'nested/new.txt',
    oldPath: null,
    newPath: 'nested/new.txt',
    oldText: '',
    newText: 'new\nfile\n',
  })

  const existing = await env.exec({ shellId: created.shellId, script: 'cat existing.txt' })
  expect(existing.output.stdoutDelta).toBe('changed\n')
  const added = await env.exec({ shellId: created.shellId, script: 'cat nested/new.txt' })
  expect(added.output.stdoutDelta).toBe('new\nfile\n')
})

test('editor patch deletes files with a /dev/null target', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create doomed.txt <<'EOF'\nremove\nEOF",
  })

  const patched = await env.exec({
    shellId: created.shellId,
    script: "editor patch <<'PATCH'\n--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-remove\nPATCH",
  })
  expect(patched.output.stdoutDelta).toBe('Patched 1 file(s)\n')
  expect(editorDiffs(patched)[0]).toMatchObject({
    action: 'delete',
    path: 'doomed.txt',
    oldPath: 'doomed.txt',
    newPath: null,
    oldText: 'remove\n',
    newText: '',
  })

  const missing = await env.exec({ shellId: created.shellId, script: 'test ! -e doomed.txt' })
  expect(missing.status).toBe('exited')
  if (missing.status !== 'exited') throw new Error('expected exited result')
  expect(missing.exitCode).toBe(0)
})

test('editor patch validates all files before writing any changes', async () => {
  const { env } = await createEditorEnvironment()

  const created = await env.exec({
    script: "editor create first.txt <<'EOF'\nfirst\nEOF\neditor create second.txt <<'EOF'\nsecond\nEOF",
  })

  const failed = await env.exec({
    shellId: created.shellId,
    script:
      "editor patch <<'PATCH'\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-wrong\n+changed\nPATCH",
  })
  expect(failed.status).toBe('exited')
  if (failed.status !== 'exited') throw new Error('expected exited result')
  expect(failed.exitCode).toBe(1)
  expect(failed.output.stderrDelta).toContain('Patch does not apply to second.txt')

  const first = await env.exec({ shellId: created.shellId, script: 'cat first.txt' })
  expect(first.output.stdoutDelta).toBe('first\n')
})

test('editor patch rolls back files when a later write fails', async () => {
  const host = new FailingWriteHost('/workspace', {
    'first.txt': 'first\n',
    'second.txt': 'second\n',
  })
  const command = createEditorCommand(host)
  const patch = command.subcommands.find((subcommand) => subcommand.name === 'patch')
  if (!patch) throw new Error('missing editor patch command')
  const output = commandOutput()

  const result = await patch.run({
    argv: ['editor', 'patch'],
    parsed: {
      subcommand: 'patch',
      json: false,
      values: {
        patch:
          '--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-second\n+changed\n',
      },
    },
    stdin: { text: '' },
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

async function createEditorEnvironment(): Promise<{ env: BashEnvironment }> {
  const root = await mkdtemp(join(tmpdir(), 'demi-editor-'))
  const host = new LocalHost(root)
  const env = new BashEnvironment({
    host,
    commands: createCodingCommandRegistry({ editorHost: host }),
    shellIdFactory: () => 'editor-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  return { env }
}

function editorDiffs(result: { status: string; commandMetadata?: Array<{ metadata: unknown }> }): Record<string, unknown>[] {
  if (result.status !== 'exited') throw new Error('expected exited result')
  const metadata = result.commandMetadata?.[0]?.metadata
  if (!isRecord(metadata) || metadata.type !== 'editor_file_diffs' || !Array.isArray(metadata.diffs)) {
    throw new Error('missing editor file diff metadata')
  }
  return metadata.diffs.map((diff) => {
    if (!isRecord(diff)) throw new Error('invalid editor diff metadata')
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
  private readonly files = new Map<string, string>()

  constructor(
    readonly root: string,
    files: Record<string, string>,
  ) {
    for (const [path, content] of Object.entries(files)) {
      this.files.set(this.resolve(path), content)
    }
  }

  read(path: string): string | undefined {
    return this.files.get(this.resolve(path))
  }

  async spawn(params: HostSpawnParams): Promise<HostSpawnHandle> {
    let stdin = ''
    let result: Promise<HostCommandResult> | null = null
    const finish = async () => {
      result ??= Promise.resolve(this.run(params, stdin))
      return result
    }
    return {
      stdout: deferredBytes(async () => (await finish()).stdout),
      stderr: deferredBytes(async () => (await finish()).stderr),
      writeStdin: async (data) => {
        stdin += text(data)
      },
      closeStdin: async () => {},
      kill: async () => {},
      wait: async () => ({ exitCode: (await finish()).exitCode }),
    }
  }

  private run(params: HostSpawnParams, stdin: string): HostCommandResult {
    const args = params.args ?? []
    if (params.command === 'cat') {
      const path = this.resolve(requiredArg(args, 0))
      const content = this.files.get(path)
      return content === undefined
        ? { stdout: '', stderr: `cat: ${args[0]}: No such file or directory\n`, exitCode: 1 }
        : { stdout: content, stderr: '', exitCode: 0 }
    }

    if (params.command === 'test' && args[0] === '-e') {
      return { stdout: '', stderr: '', exitCode: this.files.has(this.resolve(requiredArg(args, 1))) ? 0 : 1 }
    }

    if (params.command === 'mkdir') {
      return { stdout: '', stderr: '', exitCode: 0 }
    }

    if (params.command === 'rm') {
      const path = this.resolve(requiredArg(args, args.length - 1))
      this.files.delete(path)
      return { stdout: '', stderr: '', exitCode: 0 }
    }

    if (params.command === 'tee') {
      const pathArg = requiredArg(args, args.length - 1)
      const path = this.resolve(pathArg)
      this.files.set(path, stdin)
      if (pathArg === 'second.txt') {
        return { stdout: stdin, stderr: `tee: ${pathArg}: simulated write failure\n`, exitCode: 1 }
      }
      return { stdout: stdin, stderr: '', exitCode: 0 }
    }

    return { stdout: '', stderr: `unsupported command: ${params.command}\n`, exitCode: 127 }
  }

  private resolve(path: string): string {
    if (path.startsWith('/')) return path
    return `${this.root}/${path}`
  }
}

interface HostCommandResult {
  stdout: string
  stderr: string
  exitCode: number
}

function requiredArg(args: string[], index: number): string {
  const value = args[index]
  if (value === undefined) throw new Error(`missing argument ${index}`)
  return value
}

async function* deferredBytes(read: () => Promise<string>): AsyncIterable<Uint8Array> {
  const value = await read()
  if (value.length > 0) yield new TextEncoder().encode(value)
}

function text(data: string | Uint8Array): string {
  return typeof data === 'string' ? data : new TextDecoder().decode(data)
}
