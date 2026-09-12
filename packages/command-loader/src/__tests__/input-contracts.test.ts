import { expect, test } from 'bun:test'
import { z } from 'zod'
import {
  CommandRegistry,
  parseCommandInput,
  renderCommandHelp,
  runtimeModule,
  validateCommandValues,
  type CommandLeaf,
  type Host,
} from '@demicodes/shell'
import { memoryCommandStorage } from '@demicodes/shell/testing'
import { emptyByteStream } from '@demicodes/utils'
import {
  buildManifest, createLoader, inMemorySource, inProcessRpc, parseManifest, treeFromManifest,
} from '../index'

function command(input: Record<string, z.ZodType>): CommandLeaf {
  return { name: 'check', summary: 'Check values.', kind: 'rpc', input, run: () => ({ exitCode: 0 }) }
}

async function roundTrip(original: CommandLeaf): Promise<CommandLeaf> {
  const manifest = await buildManifest([original], { transpile: (source) => source })
  const wire: unknown = JSON.parse(JSON.stringify(manifest))
  return treeFromManifest(parseManifest(wire), undefined)[0] as CommandLeaf
}

test('CLI conversion is identical before and after the manifest boundary', async () => {
  const original = command({
    count: z.number().int().min(1).max(20).nullable(),
    values: z.array(z.number().int()).optional(),
    enabled: z.boolean().optional(),
    mode: z.enum(['read', 'write']).optional(),
    version: z.literal(2).optional(),
  })
  const rebuilt = await roundTrip(original)
  const fixtures = [
    {
      argv: ['--count', '12', '--values', '12', '--values', '13', '--enabled', 'false', '--version', '2'],
      expected: { count: 12, values: [12, 13], enabled: false, mode: undefined, version: 2 },
    },
    {
      argv: ['--count', '1', '--enabled', '--mode', 'read'],
      expected: { count: 1, values: undefined, enabled: true, mode: 'read', version: undefined },
    },
  ]
  for (const tree of [original, rebuilt]) {
    for (const fixture of fixtures) {
      expect(parseCommandInput(tree, ['check', ...fixture.argv], '').values).toEqual(fixture.expected)
    }
    for (const argv of [
      ['--count', ''], ['--count', 'NaN'], ['--count', '1.5'], ['--count', '21'],
      ['--count', '1', '--values', 'bad'], ['--count', '1', '--enabled', 'FALSE'],
      ['--count', '1', '--version', '3'],
    ]) {
      expect(() => parseCommandInput(tree, ['check', ...argv], '')).toThrow()
    }
  }
  expect(renderCommandHelp(rebuilt)).toBe(renderCommandHelp(original))
})

test('typed command values preserve nullable arrays and reject CLI representations', async () => {
  const original = command({ values: z.array(z.string()).nullable(), enabled: z.boolean() })
  const rebuilt = await roundTrip(original)
  for (const tree of [original, rebuilt]) {
    expect(validateCommandValues(tree.input!, { values: null, enabled: false }))
      .toEqual({ values: null, enabled: false })
    for (const input of [
      { values: ['x'], enabled: 'false' },
      { values: 'x', enabled: false },
      { values: null, enabled: false, extra: 1 },
    ]) {
      expect(() => validateCommandValues(tree.input!, input)).toThrow()
    }
  }
})

test('RPC refuses wrong types before invoking the handler', async () => {
  let calls = 0
  const root: CommandLeaf = {
    ...command({ count: z.number(), enabled: z.boolean() }),
    kind: 'rpc',
    run: () => {
      calls += 1
      return { exitCode: 0 }
    },
  }
  const rpc = inProcessRpc([root], { storage: memoryCommandStorage(), host: {} as Host })
  const invocation = {
    root: 'check', path: ['check'], argv: ['check'], json: false,
    stdin: null, cwd: '/', env: {},
    io: { stdout: () => {}, stderr: () => {} },
    signal: new AbortController().signal,
    stdinStream: emptyByteStream(),
  }
  await expect(rpc({ ...invocation, args: { count: '12', enabled: 'false' } })).rejects.toThrow()
  expect(calls).toBe(0)
  expect(await rpc({ ...invocation, args: { count: 12, enabled: false } })).toEqual({ exitCode: 0 })
  expect(calls).toBe(1)
})

test('lossy command schemas fail registration and manifest construction', async () => {
  const unsupported = [
    z.number().refine((value) => value % 2 === 0),
    z.number().default(2),
    z.coerce.number(),
    z.preprocess(Number, z.number()),
    z.string().transform(Number),
    z.string().trim(),
    z.array(z.number().refine((value) => value > 0)),
    z.array(z.array(z.string())),
    z.union([z.string(), z.number()]),
  ]
  for (const schema of unsupported) {
    const root = command({ value: schema })
    expect(() => new CommandRegistry().register(root)).toThrow('check.value')
    await expect(buildManifest([root], { transpile: (source) => source }))
      .rejects.toThrow('check.value')
  }
})

test('runtime dispatch uses converted and validated manifest values', async () => {
  const root: CommandLeaf = {
    name: 'runtime', summary: 'Runtime values.', kind: 'runtime',
    input: { values: z.array(z.number().int()) },
    module: runtimeModule('export default (ctx) => { ctx.stdout(JSON.stringify(ctx.args)); return { exitCode: 0 }; }'),
  }
  const manifest = await buildManifest([root], { transpile: (source) => source })
  const loader = await createLoader({ source: inMemorySource(manifest), host: {} as Host })
  let stdout = ''
  let stderr = ''
  const io = {
    cwd: '/', env: {}, stdin: emptyByteStream(),
    stdout: (data: Uint8Array | string) => {
      stdout += typeof data === 'string' ? data : new TextDecoder().decode(data)
    },
    stderr: (data: Uint8Array | string) => {
      stderr += typeof data === 'string' ? data : new TextDecoder().decode(data)
    },
  }
  expect(await loader.dispatch('runtime', ['--values', '12', '--values', '13'], io)).toBe(0)
  expect(stdout).toBe('{"values":[12,13]}')
  stdout = ''
  expect(await loader.dispatch('runtime', ['--values', 'bad'], io)).toBe(1)
  expect(stdout).toBe('')
  expect(stderr).toContain('Invalid value for "values"')
})


test('manifest ingress refuses unknown or unsupported validation keywords', async () => {
  const manifest = await buildManifest([command({ count: z.number() })], {
    transpile: (source) => source,
  })
  for (const invalid of [
    { type: 'number', default: 2 },
    { type: 'number', customEven: true },
    { type: 'string', pattern: '^x' },
    { type: 'array', items: { type: 'number', customEven: true } },
  ]) {
    const wire = structuredClone(manifest)
    const leaf = wire.roots.check!.tree
    if ('subcommands' in leaf) {
      throw new Error('expected a leaf')
    }
    leaf.input = { type: 'object', properties: { count: invalid }, required: ['count'] }
    expect(() => parseManifest(wire)).toThrow()
  }
})


test('literal and enum syntax survives all scalar representations', async () => {
  const fixtures = [
    { schema: z.literal(true), token: 'true', expected: true },
    { schema: z.literal(false), token: 'false', expected: false },
    { schema: z.literal([true, false]), token: 'false', expected: false },
    { schema: z.literal([1, 2]), token: '2', expected: 2 },
    { schema: z.enum({ one: 1, two: 2 }), token: '2', expected: 2 },
    { schema: z.literal(['a', 'b']), token: 'b', expected: 'b' },
  ]
  for (const fixture of fixtures) {
    const original = command({ value: fixture.schema })
    const rebuilt = await roundTrip(original)
    expect(renderCommandHelp(rebuilt)).toBe(renderCommandHelp(original))
    for (const tree of [original, rebuilt]) {
      expect(parseCommandInput(tree, ['check', '--value', fixture.token], '').values)
        .toEqual({ value: fixture.expected })
      expect(() => parseCommandInput(tree, ['check', '--value', 'bad'], '')).toThrow()
    }
  }
})
