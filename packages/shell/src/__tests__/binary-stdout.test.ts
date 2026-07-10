import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { BashEnvironment, CommandRegistry, type Command } from '../index'
import { LocalHost } from '@demicodes/host-local'

// Deliberately invalid UTF-8 (PNG magic) followed by opaque bytes.
const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01])

const emitSpec: Command = {
  name: 'emit',
  summary: 'Emit test streams.',
  subcommands: [
    {
      name: 'binary',
      summary: 'Write raw binary bytes to stdout.',
      examples: [],
      run: async (ctx) => {
        await ctx.io.stdout(PNG_BYTES)
        return { exitCode: 0 }
      },
    },
    {
      name: 'text',
      summary: 'Write multibyte UTF-8 text to stdout as bytes.',
      examples: [],
      run: async (ctx) => {
        await ctx.io.stdout(new TextEncoder().encode('你好 emoji 🎉\n'))
        return { exitCode: 0 }
      },
    },
  ],
}

function makeEnv(root: string, shellId: string): BashEnvironment {
  const commands = new CommandRegistry()
  commands.register(emitSpec)
  return new BashEnvironment({
    host: new LocalHost(root),
    commands,
    shellIdFactory: () => shellId,
    initialEnv: { PATH: process.env.PATH ?? '' },
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
  expect(result.stdout.delta).toBe(`<binary stdout: ${PNG_BYTES.length} bytes>\n`)
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

test('a binary stream over the output limit is capped and marked truncated', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-binary-trunc-'))
  const env = makeEnv(root, 'shell-binary-trunc')

  const result = await env.exec({ script: 'emit binary', maxOutputBytes: 4 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.binaryStdout?.truncated).toBe(true)
  expect(result.binaryStdout?.data).toEqual(PNG_BYTES.slice(0, 4))
  expect(result.binaryStdout?.totalBytes).toBe(PNG_BYTES.length)
  // The 4-byte budget also caps the immediate view; a default-budget status
  // read shows the full placeholder.
  const view = await env.status({ commandId: result.commandId })
  if (view.status !== 'exited') throw new Error('expected exited status')
  expect(view.stdout.tail).toContain('exceeds the 4-byte output limit')
})

test('real-process (hostSpawn) output and stdin are byte-clean end to end', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-binary-spawn-'))
  const env = makeEnv(root, 'shell-binary-spawn')

  // Real OS process producing invalid-UTF-8 bytes on stdout (sh is real-spawned).
  const produced = await env.exec({ script: `sh -c "printf '\\x89PNG\\x0d\\x0a\\x1a\\x0a\\x00\\xff\\xfe'"` })
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
