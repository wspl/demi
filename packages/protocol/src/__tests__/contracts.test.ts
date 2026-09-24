import { describe, expect, test } from 'bun:test'
import { resolve } from 'node:path'
import { z } from 'zod'
import {
  blockSchema,
  clientFrameSchema,
  previewMediaType,
  serverFrameSchema,
  showsInPlace,
} from '../index'

// The Rust contract tests' fixtures and cases: the generated schemas must
// accept and refuse what the Rust types accept and refuse, except where the
// browser's schema is tolerant or a rule is Rust's alone (`contracts.md`
// § Strict and tolerant objects, § Rules only Rust checks).
const crates = resolve(import.meta.dir, '../../../../crates')

const jsonSchema = z.json()
type Json = z.infer<typeof jsonSchema>

const mutationSchema = z.strictObject({
  why: z.string(),
  fixture: z.string().optional(),
  value: jsonSchema.optional(),
  remove: z.array(z.string()).optional(),
  set: z.record(z.string(), jsonSchema).optional(),
  browserRefuses: z.boolean().optional(),
})
type Mutation = z.infer<typeof mutationSchema>

const tableSchema = z.strictObject({
  refused: z.array(mutationSchema),
  accepted: z.array(mutationSchema).optional(),
})
type Table = z.infer<typeof tableSchema>

async function read<T>(path: string, schema: z.ZodType<T>): Promise<T> {
  return schema.parse(await Bun.file(resolve(crates, path)).json())
}

function isObject(value: Json | undefined): value is { [key: string]: Json } {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** The parent of the value at a JSON pointer, and the last key. */
function parentOf(root: Json, pointer: string): [Json[] | { [key: string]: Json }, string] {
  const keys = pointer.split('/').slice(1)
  const last = keys.pop()
  if (last === undefined) {
    throw new Error(`${pointer} names the root`)
  }
  let parent: Json | undefined = root
  for (const key of keys) {
    parent = Array.isArray(parent) ? parent[Number(key)] : isObject(parent) ? parent[key] : undefined
  }
  if (!Array.isArray(parent) && !isObject(parent)) {
    throw new Error(`${pointer} is not inside an object or array`)
  }
  return [parent, last]
}

/** The value a case describes: its own, or its fixture with fields removed and set. */
function mutated(fixtures: Json[], key: string, mutation: Mutation): Json {
  if (mutation.value !== undefined) {
    return mutation.value
  }
  const base = fixtures.find((candidate) => isObject(candidate) && candidate[key] === mutation.fixture)
  if (base === undefined) {
    throw new Error(`no fixture ${mutation.fixture}`)
  }
  const value = structuredClone(base)
  for (const pointer of mutation.remove ?? []) {
    const [parent, last] = parentOf(value, pointer)
    if (Array.isArray(parent)) {
      parent.splice(Number(last), 1)
    } else {
      delete parent[last]
    }
  }
  for (const [pointer, field] of Object.entries(mutation.set ?? {})) {
    const [parent, last] = parentOf(value, pointer)
    if (Array.isArray(parent)) {
      parent[Number(last)] = field
    } else {
      parent[last] = field
    }
  }
  return value
}

function checkTable(schema: z.ZodType, fixtures: Json[], key: string, table: Table): void {
  for (const mutation of table.refused) {
    const result = schema.safeParse(mutated(fixtures, key, mutation))
    expect({ why: mutation.why, refused: !result.success }).toEqual({
      why: mutation.why,
      refused: mutation.browserRefuses ?? true,
    })
  }
  for (const mutation of table.accepted ?? []) {
    const result = schema.safeParse(mutated(fixtures, key, mutation))
    expect({ why: mutation.why, error: result.error?.issues ?? null }).toEqual({ why: mutation.why, error: null })
  }
}

const blocks = await read('core/tests/core/fixtures/blocks.json', z.array(jsonSchema))
const blockTable = await read('core/tests/core/fixtures/blocks-mutations.json', tableSchema)
const clientFrames = await read('agent-protocol/tests/frames/fixtures/client-frames.json', z.array(jsonSchema))
const clientFrameTable = await read('agent-protocol/tests/frames/fixtures/client-frames-mutations.json', tableSchema)
const serverFrames = await read('agent-protocol/tests/frames/fixtures/server-frames.json', z.array(jsonSchema))
const fileTypes = await read('core/tests/core/fixtures/file-types.json', z.strictObject({
  previewMediaType: z.array(z.tuple([z.string(), z.string().nullable()])),
  showsInPlace: z.array(z.tuple([z.string(), z.boolean()])),
}))

describe('blocks', () => {
  test('every block kind keeps its wire shape', () => {
    for (const block of blocks) {
      expect<unknown>(blockSchema.parse(block)).toEqual(block)
    }
  })

  test('the browser refuses what the Rust decode refuses, except unknown fields and Rust-only rules', () => {
    checkTable(blockSchema, blocks, 'id', blockTable)
  })
})

describe('client frames', () => {
  test('every client frame keeps its wire shape', () => {
    for (const frame of clientFrames) {
      expect<unknown>(clientFrameSchema.parse(frame)).toEqual(frame)
    }
  })

  test('the browser refuses what the backend refuses, except the content rules Rust alone checks', () => {
    checkTable(clientFrameSchema, clientFrames, 'type', clientFrameTable)
  })
})

describe('server frames', () => {
  test('every server frame keeps its wire shape', () => {
    for (const frame of serverFrames) {
      expect<unknown>(serverFrameSchema.parse(frame)).toEqual(frame)
    }
  })

  test('a page accepts frame fields it does not know and drops them', () => {
    const frame = { type: 'phase', phase: 'idle', since: '2026-09-21T14:13:20.000Z' }
    expect(serverFrameSchema.parse(frame)).toEqual({ type: 'phase', phase: 'idle' })
    expect(serverFrameSchema.safeParse({ type: 'tool_progress', toolUseId: 't1' }).success).toBe(false)
  })
})

describe('file types', () => {
  test('a file is known by its extension, whatever its ASCII case or separator', () => {
    for (const [path, expected] of fileTypes.previewMediaType) {
      expect({ path, type: previewMediaType(path) }).toEqual({ path, type: expected })
    }
  })

  test('the page shows media in place and renders Markdown from its text', () => {
    for (const [mediaType, expected] of fileTypes.showsInPlace) {
      expect({ mediaType, inPlace: showsInPlace(mediaType) }).toEqual({ mediaType, inPlace: expected })
    }
  })
})
