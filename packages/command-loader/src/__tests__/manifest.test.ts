import { describe, expect, test } from 'bun:test'
import { isCommandGroup, renderCommandHelp, runtimeModule, type Command } from '@demicodes/shell'
import { z } from 'zod'
import { buildManifest, isManifestGroup, parseManifest, treeFromManifest } from '../index'
import { COPY_MODULE, testRoots, transpile } from './fixtures'

describe('buildManifest', () => {
  test('hashes are a function of content: two builds of the same trees agree', async () => {
    const a = await buildManifest(testRoots(), { transpile })
    const b = await buildManifest(testRoots(), { transpile })
    expect(a).toEqual(b)
    expect(Object.keys(a.modules)).toHaveLength(2)
    expect(a.hash).toMatch(/^[0-9a-f]{64}$/)
  })

  test('a different module is a different manifest hash; a comment-only change is not', async () => {
    const a = await buildManifest(testRoots(), { transpile })
    const roots = testRoots()
    const myagent = roots[0]!
    if (!isCommandGroup(myagent)) throw new Error('fixture')
    const copy = myagent.subcommands[0]!
    if (isCommandGroup(copy) || copy.kind !== 'runtime') throw new Error('fixture')
    copy.module = runtimeModule(`${COPY_MODULE}\n// a comment transpiles away\n`)
    expect((await buildManifest(roots, { transpile })).hash).toBe(a.hash)
    copy.module = runtimeModule(COPY_MODULE.replace('copied', 'duplicated'))
    expect((await buildManifest(roots, { transpile })).hash).not.toBe(a.hash)
  })

  test('the tree carries kinds, help, positionals and JSON Schema with path marks', async () => {
    const manifest = await buildManifest(testRoots(), { transpile })
    const root = manifest.roots.myagent!.tree
    if (!isManifestGroup(root)) throw new Error('root')
    const copy = root.subcommands[0]!
    if (isManifestGroup(copy)) throw new Error('copy')
    expect(copy.kind).toBe('runtime')
    expect(copy.positionals).toEqual(['from', 'to'])
    expect(copy.module).toBeDefined()
    expect(manifest.modules[copy.module!]).toContain('export default')
    const input = copy.input as { properties: Record<string, Record<string, unknown>>; required: string[] }
    expect(input.properties.from).toMatchObject({ type: 'string', path: true, description: 'Source path' })
    expect(input.properties.upper).toMatchObject({ type: 'boolean' })
    expect(input.required).toEqual(['from', 'to'])
    const note = root.subcommands[2]!
    if (!isManifestGroup(note)) throw new Error('note')
    const add = note.subcommands[0]!
    if (isManifestGroup(add)) throw new Error('add')
    expect(add.kind).toBe('rpc')
    expect(add.stdinField).toBe('text')
    expect(add.output?.json).toMatchObject({ type: 'object' })
  })

  test('a module that imports a value fails the build', async () => {
    const roots: Command[] = [
      {
        name: 'bad',
        summary: 'x',
        subcommands: [
          {
            name: 'leaf',
            kind: 'runtime',
            summary: 'x',
            module: runtimeModule("import { join } from 'node:path'\nexport default async () => ({ exitCode: 0 })\n"),
          },
        ],
      },
    ]
    await expect(buildManifest(roots, { transpile })).rejects.toThrow('imports a value')
  })

  test('duplicate roots are refused', async () => {
    await expect(buildManifest([...testRoots(), ...testRoots()], { transpile })).rejects.toThrow('duplicate root')
  })

  test('the manifest survives JSON and parses back', async () => {
    const manifest = await buildManifest(testRoots(), { transpile })
    expect(parseManifest(JSON.parse(JSON.stringify(manifest)))).toEqual(manifest)
    expect(() => parseManifest({ hash: 'x' })).toThrow()
  })
})

describe('treeFromManifest', () => {
  test('help rendered from the reconstructed tree equals help from the declared tree', async () => {
    const declared = testRoots()
    const manifest = await buildManifest(declared, { transpile })
    const rebuilt = treeFromManifest(manifest, undefined)
    expect(rebuilt.map((root) => renderCommandHelp(root))).toEqual(declared.map((root) => renderCommandHelp(root)))
  })

  test('input schemas validate as declared after the round trip', async () => {
    const manifest = await buildManifest(testRoots(), { transpile })
    const [root] = treeFromManifest(manifest, undefined)
    if (!root || !isCommandGroup(root)) throw new Error('root')
    const copy = root.subcommands[0]!
    if (isCommandGroup(copy)) throw new Error('copy')
    expect(copy.input!.upper!.safeParse(undefined).success).toBe(true)
    expect(copy.input!.from!.safeParse(undefined).success).toBe(false)
    const echo = root.subcommands[1]!
    if (isCommandGroup(echo)) throw new Error('echo')
    expect(echo.input!.code!.safeParse(2.5).success).toBe(false)
    expect(echo.input!.code!.safeParse(3).success).toBe(true)
    expect(z.object(copy.input!).safeParse({ from: 'a', to: 'b', upper: true }).success).toBe(true)
  })
})
