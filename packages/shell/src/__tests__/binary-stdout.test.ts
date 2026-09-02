import { access, mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { BashEnvironment, CommandRegistry, type Command } from '../index'
import { LocalHost } from '@demicodes/host-local'

// Deliberately invalid UTF-8 (PNG magic) followed by opaque bytes.
const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01])
// Invalid UTF-8 that matches no media magic — the model cannot look at it.
const OPAQUE_BYTES = new Uint8Array([0xff, 0xfe, 0xfd, 0xfc, 0xfb, 0xfa, 0xf9, 0xf8, 0xf7, 0xf6, 0xf5, 0xf4])

const emitSpec: Command = {
  name: 'emit',
  summary: 'Emit test streams.',
  subcommands: [
    {
      name: 'binary',
      summary: 'Write raw binary bytes to stdout.',
      kind: 'rpc',
      run: async (ctx) => {
        await ctx.io.stdout(PNG_BYTES)
        return { exitCode: 0 }
      },
    },
    {
      name: 'opaque',
      summary: 'Write raw bytes that match no media magic.',
      kind: 'rpc',
      run: async (ctx) => {
        await ctx.io.stdout(OPAQUE_BYTES)
        return { exitCode: 0 }
      },
    },
    {
      name: 'text',
      summary: 'Write multibyte UTF-8 text to stdout as bytes.',
      kind: 'rpc',
      run: async (ctx) => {
        await ctx.io.stdout(new TextEncoder().encode('你好 emoji 🎉\n'))
        return { exitCode: 0 }
      },
    },
  ],
}

function makeEnv(root: string, shellId: string, extra: { maxBinaryBytes?: number } = {}): BashEnvironment {
  const commands = new CommandRegistry()
  commands.register(emitSpec)
  return new BashEnvironment({
    host: new LocalHost(root),
    commands,
    shellIdFactory: () => shellId,
    initialEnv: { PATH: process.env.PATH ?? '' },
    ...extra,
  })
}

test('a binary final stream surfaces as binaryStdout with a placeholder text render', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-binary-'))
  const env = makeEnv(root, 'shell-binary')

  const result = await env.exec({ script: 'emit binary' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')

  expect(result.binaryStdout?.data).toEqual(PNG_BYTES)
  expect(result.binaryStdout?.truncated).toBe(false)
  expect(result.binaryStdout?.totalBytes).toBe(PNG_BYTES.length)
  expect(result.stdout.delta).toBe(
    `<binary stdout: ${PNG_BYTES.length} bytes; raw bytes at ${result.artifactDir}/stdout.bin>\n`,
  )

  // The raw bytes stay addressable as a plain file on disk (written async).
  await waitForFile(`${result.artifactDir}/stdout.bin`)
  const counted = await env.exec({
    shellId: result.shellId,
    script: `wc -c < ${result.artifactDir}/stdout.bin`,
  })
  if (counted.status !== 'exited') throw new Error('expected exited result')
  expect(counted.stdout.delta.trim()).toBe(String(PNG_BYTES.length))
})

test('byte output that is valid UTF-8 stays text, byte-identical through the pipe', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-binary-text-'))
  const env = makeEnv(root, 'shell-binary-text')

  const result = await env.exec({ script: 'emit text' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.binaryStdout).toBeUndefined()
  expect(result.stdout.delta).toBe('你好 emoji 🎉\n')
})

test('binary streams pipe byte-clean into downstream fork commands', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-binary-pipe-'))
  const env = makeEnv(root, 'shell-binary-pipe')

  const counted = await env.exec({ script: 'emit binary | wc -c' })
  expect(counted.status).toBe('exited')
  if (counted.status !== 'exited') throw new Error('expected exited result')
  expect(counted.binaryStdout).toBeUndefined()
  expect(counted.stdout.delta.trim()).toBe(String(PNG_BYTES.length))

  const encoded = await env.exec({ script: 'emit binary | base64' })
  expect(encoded.status).toBe('exited')
  if (encoded.status !== 'exited') throw new Error('expected exited result')
  const roundTripped = Uint8Array.from(Buffer.from(encoded.stdout.delta.replaceAll('\n', ''), 'base64'))
  expect(roundTripped).toEqual(PNG_BYTES)
})

test('the text output limit does not decide whether raw bytes survive', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-binary-text-budget-'))
  const env = makeEnv(root, 'shell-binary-text-budget')

  // maxOutputBytes exists to stop a log flood; bytes carried to be LOOKED at
  // answer to their own ceiling, so a tight text budget must leave them whole.
  const kept = await env.exec({ script: 'emit binary', maxOutputBytes: 4 })
  if (kept.status !== 'exited') throw new Error('expected exited result')
  expect(kept.binaryStdout?.truncated).toBe(false)
  expect(kept.binaryStdout?.data).toEqual(PNG_BYTES)
})

test('a binary stream over the binary ceiling is capped and names that ceiling', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-binary-trunc-'))
  const env = makeEnv(root, 'shell-binary-trunc', { maxBinaryBytes: 4 })

  const result = await env.exec({ script: 'emit binary' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.binaryStdout?.truncated).toBe(true)
  expect(result.binaryStdout?.limitBytes).toBe(4)
  expect(result.binaryStdout?.data).toEqual(PNG_BYTES.slice(0, 4))
  expect(result.binaryStdout?.totalBytes).toBe(PNG_BYTES.length)
  const view = await env.status({ commandId: result.commandId })
  if (view.status !== 'exited') throw new Error('expected exited status')
  expect(view.stdout.tail).toContain('exceeds the 4-byte binary limit')
})

test('real-process (hostSpawn) output and stdin are byte-clean end to end', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-binary-spawn-'))
  const env = makeEnv(root, 'shell-binary-spawn')

  // Real OS process producing invalid-UTF-8 bytes on stdout (sh is real-spawned).
  // Octal escapes, not \xHH: POSIX printf takes octal everywhere while \xHH is
  // a bashism — hosts where /bin/sh is dash echo \xHH literally.
  const produced = await env.exec({ script: `sh -c "printf '\\211PNG\\015\\012\\032\\012\\000\\377\\376'"` })
  expect(produced.status).toBe('exited')
  if (produced.status !== 'exited') throw new Error('expected exited result')
  expect(produced.binaryStdout?.data).toEqual(
    new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe]),
  )

  // Binary bytes from a fork command piped INTO a real OS process arrive intact.
  const counted = await env.exec({ shellId: produced.shellId, script: 'emit binary | sh -c "wc -c"' })
  expect(counted.status).toBe('exited')
  if (counted.status !== 'exited') throw new Error('expected exited result')
  expect(counted.stdout.delta.trim()).toBe(String(PNG_BYTES.length))

  // Full round-trip through a real process: bytes out, bytes back, unchanged.
  const roundTrip = await env.exec({ shellId: produced.shellId, script: 'emit binary | sh -c cat' })
  expect(roundTrip.status).toBe('exited')
  if (roundTrip.status !== 'exited') throw new Error('expected exited result')
  expect(roundTrip.binaryStdout?.data).toEqual(PNG_BYTES)
})

async function waitForFile(path: string): Promise<void> {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    try {
      await access(path)
      return
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 5))
    }
  }
  throw new Error(`timed out waiting for artifact file ${path}`)
}
