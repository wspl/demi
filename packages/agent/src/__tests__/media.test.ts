import { expect, test } from 'bun:test'
import type { Block } from '@demicodes/core'
import type { BlobStore } from '../store/media'
import { externalizeBlockMedia, rehydrateBlockMedia } from '../store/media'
import { blockSchema } from '../protocol/schemas'

function memoryBlobs(): BlobStore {
  const bytes = new Map<string, Uint8Array>()
  return {
    put: async (data) => {
      const key = `sha-${[...data].join('-')}`
      bytes.set(key, data)
      return key
    },
    get: async (sha256) => bytes.get(sha256) ?? null,
  }
}

function userBlock(source: unknown): Block {
  return {
    type: 'user',
    id: 'u1',
    turnId: 'turn-1',
    createdAt: '2026-09-13T00:00:00.000Z',
    model: {
      providerId: 'stub',
      model: {
        id: 'stub',
        name: 'Stub',
        contextWindow: 1000,
        outputLimit: null,
        inputLimit: null,
        thinking: [],
        acceptedExtensions: [],
      },
      thinking: null,
    },
    content: [{ type: 'image', source }],
    preamble: null,
  } as unknown as Block
}

test('media survives the round trip through the blob store', async () => {
  const blobs = memoryBlobs()
  const data = new Uint8Array([1, 2, 3])
  const block = userBlock({ type: 'binary', mediaType: 'image/png', data })

  const stored = await externalizeBlockMedia(block, blobs)
  const storedSource = (stored as Extract<Block, { type: 'user' }>).content[0]
  expect(storedSource).toMatchObject({
    type: 'image',
    source: { type: 'ref', mediaType: 'image/png' },
  })

  const restored = await rehydrateBlockMedia(stored, blobs)
  expect((restored as Extract<Block, { type: 'user' }>).content[0]).toEqual({
    type: 'image',
    source: { type: 'binary', mediaType: 'image/png', data },
  })
})

test('the block schema accepts both wire forms of a media source', () => {
  const data = new Uint8Array([1, 2, 3])

  expect(
    blockSchema.safeParse(userBlock({
      type: 'binary',
      mediaType: 'image/png',
      data
    })).success,
  ).toBe(true)
  expect(
    blockSchema.safeParse(userBlock({
      type: 'ref',
      ref: 'sha-1-2-3',
      mediaType: 'image/png',
    })).success,
  ).toBe(true)
  expect(
    blockSchema.safeParse(userBlock({
      type: 'ref',
      ref: 42,
      mediaType: 'image/png'
    })).success,
  ).toBe(false)
})

test('a corrupt stored reference is reported, not quietly carried on', () => {
  const blobs = memoryBlobs()
  const block = userBlock({ type: 'ref', ref: 42, mediaType: 'image/png' })

  expect(rehydrateBlockMedia(block, blobs)).rejects.toThrow()
})

test('a missing blob degrades to a placeholder instead of failing', async () => {
  const blobs = memoryBlobs()
  const block = userBlock({
    type: 'ref',
    ref: 'sha-gone',
    mediaType: 'image/png'
  })

  const restored = await rehydrateBlockMedia(block, blobs)

  expect((restored as Extract<Block, { type: 'user' }>).content[0]).toEqual({
    type: 'text',
    text: '[missing image blob sha-gone]',
  })
})
