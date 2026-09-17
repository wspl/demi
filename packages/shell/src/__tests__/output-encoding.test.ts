import { mkdtemp, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { LocalHost } from '@demicodes/host-local'
import { BashEnvironment, CommandRegistry } from '../index'

const unicode = 'Latest·No.865 café Ã© 中文 🎉\uFEFF'
const opaque = new Uint8Array([0x89, 0x50, 0, 0xff, 0xc2])

async function environment() {
  const root = await mkdtemp(join(tmpdir(), 'demi-output-encoding-'))
  const commands = new CommandRegistry()
  for (const [index, value] of ['Latest·No.865', 'café', 'Ã©'].entries()) {
    commands.register({ name: `latin-${index}`, summary: 'Latin text fixture', run: async (ctx) => {
      await ctx.io.stdout(value)
      return { exitCode: 0 }
    } })
  }
  commands.register({ name: 'emit-text', summary: 'Text fixture', run: async (ctx) => {
    await ctx.io.stdout(unicode)
    return { exitCode: 0 }
  } })
  commands.register({ name: 'emit-bytes', summary: 'Byte fixture', run: async (ctx) => {
    await ctx.io.stdout(opaque)
    return { exitCode: 7 }
  } })
  commands.register({ name: 'copy-input', summary: 'Copy bytes unchanged', run: async (ctx) => {
    await ctx.io.stdout(ctx.stdin.bytes)
    return { exitCode: 0 }
  } })
  commands.register({ name: 'split-text', summary: 'Split every UTF-8 character across chunks', run: async (ctx) => {
    for (const byte of new TextEncoder().encode(unicode)) {
      await ctx.io.stdout(new Uint8Array([byte]))
      await ctx.io.stderr(new Uint8Array([byte]))
    }
    return { exitCode: 0 }
  } })
  return { root, env: new BashEnvironment({ host: new LocalHost(root), commands }) }
}

for (const [index, value] of ['Latest·No.865', 'café', 'Ã©'].entries()) {
  test(`Latin-only output remains exactly ${value}`, async () => {
    const { env } = await environment()
    const result = await env.exec({ script: `latin-${index}` })
    expect(result.stdout.delta).toBe(value)
    expect(result.output.text).toBe(value)
  })
}

for (const script of ['emit-text', 'emit-text | copy-input', '{ emit-text; }', '(emit-text)', 'for i in 1; do emit-text; done', 'f() { emit-text; }; f']) {
  test(`Unicode is preserved through ${script}`, async () => {
    const { env } = await environment()
    const result = await env.exec({ script })
    expect(result.status).toBe('exited')
    expect(result.stdout.delta).toBe(unicode)
    expect(result.output.text).toBe(unicode)
    if (result.status === 'exited') expect(result.binaryStdout).toBeUndefined()
  })
}

for (const script of ['emit-bytes', 'emit-bytes | copy-input', '{ emit-bytes; }', 'for i in 1; do emit-bytes; done', 'emit-bytes; exit 7']) {
  test(`Opaque bytes are preserved through ${script}`, async () => {
    const { env } = await environment()
    const result = await env.exec({ script })
    expect(result.status).toBe('exited')
    if (result.status !== 'exited') throw new Error('expected exit')
    expect(result.binaryStdout?.data).toEqual(opaque)
  })
}

test('mixed text and bytes preserve every byte through statement aggregation and redirection', async () => {
  const { env, root } = await environment()
  const result = await env.exec({ script: '{ emit-text; emit-bytes; } > result.bin' })
  expect(result.status).toBe('exited')
  expect(result.stdout.delta).toBe('')
  expect(new Uint8Array(await readFile(join(root, 'result.bin')))).toEqual(new Uint8Array([...new TextEncoder().encode(unicode), ...opaque]))
})

test('streamed stdout and stderr decode across chunk boundaries without replacing characters', async () => {
  const { env } = await environment()
  const result = await env.exec({ script: 'split-text' })
  expect(result.stdout.delta).toBe(unicode)
  expect(result.stderr.delta).toBe(unicode)
  expect(result.output.chunks.filter((c) => c.stream === 'stdout').map((c) => c.text).join('')).toBe(unicode)
})

test('descriptor duplication preserves Unicode and opaque bytes', async () => {
  const { env, root } = await environment()
  const textResult = await env.exec({ script: 'emit-text >&2' })
  expect(textResult.stdout.delta).toBe('')
  expect(textResult.stderr.delta).toBe(unicode)
  await env.exec({ script: '{ emit-bytes >&2; } 2> error.bin' })
  expect(new Uint8Array(await readFile(join(root, 'error.bin')))).toEqual(opaque)
})

for (const script of [
  `copy-input <<< '${unicode}'`,
  `{ copy-input; } <<< '${unicode}'`,
  `if true; then copy-input; fi <<< '${unicode}'`,
  `f() { copy-input; } <<< '${unicode}'; f`,
]) {
  test(`text input becomes UTF-8 bytes at its source: ${script}`, async () => {
    const { env } = await environment()
    expect((await env.exec({ script })).stdout.delta).toBe(`${unicode}\n`)
  })
}
