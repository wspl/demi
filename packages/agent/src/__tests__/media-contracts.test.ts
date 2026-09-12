import { expect, test } from 'bun:test'
import type { Block, UserContentBlock } from '@demicodes/core'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import { externalizeBlockMedia, rehydrateBlockMedia, type BlobStore } from '../store/media'
import {
  storedBlockSchema,
  displayedBlockSchema,
  displayedServerFrameSchema,
} from '../store/media-contracts'
import { blockSchema } from '../protocol/block-schemas'
import { model } from './helpers'

const meta = { id: 'block', createdAt: '2026-09-01T00:00:00.000Z', model }
function blobStore() {
  const values = new Map<string, Uint8Array>()
  const store: BlobStore = {
    async put(bytes) {
      const key = new Bun.CryptoHasher('sha256').update(bytes).digest('hex')
      values.set(key, new Uint8Array(bytes))
      return key
    },
    async get(key) {
      return values.get(key) ?? null
    },
  }
  return { store, values }
}

const content: UserContentBlock[] = [
  { type: 'text', text: 'with media' },
  { type: 'image', source: { type: 'binary', data: new Uint8Array([1]), mediaType: 'image/png' } },
  { type: 'video', source: { type: 'binary', data: new Uint8Array([2]), mediaType: 'video/mp4' } },
  {
    type: 'document',
    source: { data: new Uint8Array([3]), mediaType: 'application/pdf', fileName: 'sample.pdf' },
  },
  { type: 'image', source: { type: 'url', url: 'https://example.test/image.png' } },
]
const user: Block = {
  ...meta,
  type: 'user',
  turnId: 'turn',
  content,
  resolvedContent: content,
  preamble: null,
}
const tool: Block = {
  ...meta,
  type: 'tool_call',
  toolUseId: 'tool',
  toolName: 'shell_exec',
  input: '{}',
  status: 'completed',
  view: null,
  output: [{ type: 'image', source: { mediaType: 'image/png', data: 'AQ==' } }],
  streamingOutput: [{ type: 'video', source: { mediaType: 'video/mp4', data: 'Ag==' } }],
}

test('inline, stored and displayed media keep distinct contracts through portable JSON', async () => {
  const { store } = blobStore()
  for (const original of [user, tool]) {
    const stored = await externalizeBlockMedia(original, store)
    const decoded = storedBlockSchema.parse(parsePortableJson(stringifyPortableJson(stored)))
    expect(blockSchema.safeParse(decoded).success).toBe(false)
    expect(storedBlockSchema.safeParse(original).success).toBe(false)
    expect(displayedBlockSchema.parse(decoded)).toEqual(decoded)
    expect(displayedBlockSchema.parse(original)).toEqual(original)
    expect(await rehydrateBlockMedia(decoded, store)).toEqual(original)
    expect(
      displayedServerFrameSchema.parse({
        type: 'transcript_reset',
        epoch: 'e',
        revision: 1,
        blocks: [decoded],
      }),
    ).toMatchObject({ blocks: [decoded] })
  }
  const stored = await externalizeBlockMedia(tool, store)
  if (stored.type !== 'tool_call') throw new Error('Expected a tool block')
  expect(stored.output[0]).toMatchObject({ type: 'image', source: { ref: expect.any(String) } })
  expect(stored.output[0]).not.toHaveProperty('source.type')
})

test('malformed references and mixed representations fail before blob lookup', async () => {
  const source = { type: 'ref', ref: '0'.repeat(64), mediaType: 'image/png' }
  for (const invalid of [
    { ...source, ref: '' },
    { ...source, ref: '../path' },
    { ...source, mediaType: 7 },
    { ...source, data: new Uint8Array([1]) },
    { type: 'ref' },
  ]) {
    expect(
      storedBlockSchema.safeParse({
        ...user,
        content: [{ type: 'image', source: invalid }],
        resolvedContent: [],
      }).success,
    ).toBe(false)
  }
  expect(
    storedBlockSchema.safeParse({
      ...user,
      content: [{ type: 'document', source }],
      resolvedContent: [],
    }).success,
  ).toBe(false)
})

test('only absent valid blobs become placeholders; IO failures propagate', async () => {
  const { store, values } = blobStore()
  const stored = storedBlockSchema.parse(await externalizeBlockMedia(user, store))
  values.clear()
  const restored = await rehydrateBlockMedia(stored, store)
  if (restored.type !== 'user') throw new Error('Expected user block')
  expect(
    restored.content
      .slice(1, 4)
      .every((part) => part.type === 'text' && part.text.startsWith('[missing ')),
  ).toBe(true)
  expect(restored.content[4]).toEqual(content[4])
  await expect(
    rehydrateBlockMedia(stored, {
      put: store.put,
      get: async () => {
        throw new Error('blob IO failure')
      },
    }),
  ).rejects.toThrow('blob IO failure')
})
