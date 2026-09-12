import { z } from 'zod'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { fileHostStore } from '../index'
import { LocalHost } from '@demicodes/runner/testing'

const storeAt = (root: string) => fileHostStore(
  new LocalHost(root, { storeRoot: root }).fs,
  root
)

test('fileHostStore reads, writes, lists, and deletes JSON files', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = storeAt(root)

  await store.writeJson('nested/todos.json', [{ text: 'a' }])

  expect(
    await store.readJson('nested/todos.json')
  ).toEqual(
    [{
      text: 'a'
    }]
  )
  expect(await store.list('')).toEqual(['nested/todos.json'])
  expect(await store.list('nested')).toEqual(['nested/todos.json'])

  await store.delete('nested/todos.json')

  expect(await store.readJson('nested/todos.json')).toBeNull()
  expect(await store.list('')).toEqual([])
})

test(
  'fileHostStore round-trips Uint8Array values inside stored JSON',
  async () => {
    const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
    const store = storeAt(root)

    await store.writeJson('session/checkpoint.json', {
      content: [{
        type: 'image',
        source: {
          type: 'binary',
          data: new Uint8Array([137, 80, 78, 71]),
          mediaType: 'image/png'
        }
      }],
    })

    const restored = z.object({
      content: z.array(z.object({
        source: z.object({ data: z.instanceof(Uint8Array) }),
      })),
    }).parse(await store.readJson('session/checkpoint.json'))
    expect(restored?.content[0].source.data).toBeInstanceOf(Uint8Array)
    expect([...(restored?.content[0].source.data ?? [])]).toEqual([
      137,
      80,
      78,
      71
    ])
  }
)

test(
  'fileHostStore keeps a concurrently overwritten key complete and parseable',
  async () => {
    const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
    const store = storeAt(root)
    // Payloads large enough that a torn or interleaved write could not parse back.
    const payloads = Array.from(
      { length: 6 },
      (_, writer) => ({ writer, filler: `${writer}`.repeat(2_000_000) })
    )

    for (let round = 0; round < 3; round++) {
      await Promise.all(payloads.map((payload) => store.writeJson(
        'session/checkpoint.json',
        payload
      )))

      const restored = z.object({ writer: z.number(), filler: z.string() }).parse(
        await store.readJson('session/checkpoint.json'),
      )
      expect(restored.filler).toBe(`${restored.writer}`.repeat(2_000_000))
      // No temp files may survive a completed write.
      expect(await store.list('')).toEqual(['session/checkpoint.json'])
    }
  }
)

test(
  'fileHostStore rejects keys that are not relative store paths',
  async () => {
    const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
    const store = storeAt(root)

    await expect(store.writeJson('../outside.json', {}))
      .rejects.toThrow('path traversal')
    await expect(store.writeJson('nested/../inside.json', {}))
      .rejects.toThrow('path traversal')
    await expect(
      store.writeJson(join(root, 'absolute-inside-root.json'), {})
    ).rejects.toThrow('HostStore keys must be relative')
    await expect(store.writeJson('bad\0key.json', {}))
      .rejects.toThrow('Invalid HostStore key')
  }
)

test('fileHostStore distinguishes missing keys from corrupt data and IO failures', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-corruption-'))
  const store = storeAt(root)
  try {
    expect(await store.readJson('missing')).toBeNull()
    for (const payload of [
      '{',
      '{"__demiBigInt":true,"value":"oops"}',
      new Uint8Array([34, 255, 34]),
    ]) {
      await writeFile(join(root, 'bad'), payload)
      await expect(store.readJson('bad')).rejects.toThrow()
    }
    await mkdir(join(root, 'directory'))
    await expect(store.readJson('directory')).rejects.toThrow()
    await store.writeJson('existing', { count: 1 })
    await expect(store.writeJson('existing', { count: Infinity })).rejects.toThrow()
    expect(await store.readJson('existing')).toEqual({ count: 1 })
    expect((await store.list('')).some((path) => path.endsWith('.tmp'))).toBe(false)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
