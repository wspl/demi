import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import type { UserContentBlock } from '@demicodes/core'
import { World } from './world'
import { model, type Target } from './driver'

// S8 — an image attached to a user message: the model receives the bytes
// inline, the transcript stores a reference, the blob route serves it, and
// the cold transcript carries the same reference (the teardown equality).

let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'] })
})

afterAll(async () => {
  await world.close()
})

const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01])

describe.each<Target>(['hostless', 'runner:alpha'])('S8 attachments on %s', (target) => {
  test('upload → ref → bytes at the model → blob route', async () => {
    const driver = await world.conversation(target)
    const uploaded = await world.backend.session.fetch(`/api/attachments`, { method: 'POST', body: PNG_BYTES, headers: { 'content-type': 'image/png' } })
    expect(uploaded.status).toBe(201)
    const { attachment } = (await uploaded.json()) as { attachment: { id: string; sha256: string } }

    let seen: Extract<UserContentBlock, { type: 'image' }> | undefined
    const turn = await driver.turn({
      content: [{ type: 'text', text: 'describe this' }, { type: 'image', source: { type: 'ref', ref: attachment.id } } as never],
      model: [
        (request) => {
          const message = request.items.find((item) => item.type === 'user_message')
          seen = message?.type === 'user_message' ? message.content.find((block): block is Extract<UserContentBlock, { type: 'image' }> => block.type === 'image') : undefined
          return model.say('a tiny png')
        },
      ],
    })
    expect(seen?.source.type).toBe('binary')
    if (seen?.source.type !== 'binary') throw new Error('expected inline bytes at the model')
    expect(seen.source.data).toEqual(PNG_BYTES)
    expect(seen.source.mediaType).toBe('image/png')

    const user = turn.blocks.find((block) => block.type === 'user')
    const image = user?.type === 'user' ? user.content.find((block) => block.type === 'image') : undefined
    expect(image && 'source' in image ? image.source : null).toEqual({ type: 'ref', ref: attachment.sha256, mediaType: 'image/png' } as never)

    const blob = await world.backend.session.fetch(`/api/blobs/${attachment.sha256}?type=image/png`)
    expect(blob.status).toBe(200)
    expect(new Uint8Array(await blob.arrayBuffer())).toEqual(PNG_BYTES)
  }, 30_000)
})
