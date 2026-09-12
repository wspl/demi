import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import type { UserContentBlock } from '@demicodes/core'
import { World } from './world'
import { model, type Target } from './driver'

// S8 — an image attached to a user message: the model receives the bytes
// inline and the attachment record naming the file on the host, the
// transcript stores the image by reference, the blob route serves it, and
// the cold transcript carries the same reference (the teardown equality).

let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'] })
})

afterAll(async () => {
  await world.close()
})

const PNG_BYTES = new Uint8Array([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a,
  0x00,
  0xff,
  0xfe,
  0x01
])

describe.each<Target>(['cloud', 'runner:alpha'])(
  'S8 attachments on %s',
  (target) => {
    test('upload → ref → bytes at the model → blob route', async () => {
      const driver = await world.conversation(target)
      const uploaded = await world.backend.session.fetch(`/api/attachments`, {
        method: 'POST',
        body: PNG_BYTES,
        headers: { 'content-type': 'image/png' }
      })
      expect(uploaded.status).toBe(201)
      const { attachment } = (await uploaded.json()) as { attachment: {
          id: string;
          sha256: string
        } }

      let seen: UserContentBlock[] = []
      const turn = await driver.turn({
        content: [
          { type: 'text', text: 'describe this' },
          { type: 'upload', ref: attachment.id, fileName: 'tiny.png' }
        ],
        model: [
          (request) => {
            const message = request.items.find(
              (item) => item.type === 'user_message'
            )
            seen = message?.type === 'user_message' ? message.content : []
            return model.say('a tiny png')
          },
        ],
      })
      expect(seen.map((block) => block.type)).toEqual(['text', 'image', 'attachment'])
      const image = seen.find(
        (block): block is Extract<UserContentBlock, { type: 'image' }> => block.type === 'image'
      )
      if (image?.source.type !== 'binary')
        throw new Error('expected inline bytes at the model')
      expect(image.source.data).toEqual(PNG_BYTES)
      expect(image.source.mediaType).toBe('image/png')
      const record = seen.find(
        (block): block is Extract<UserContentBlock, { type: 'attachment' }> => block.type === 'attachment'
      )
      expect(record?.name).toBe('tiny.png')
      expect(record?.sha256).toBe(attachment.sha256)

      const user = turn.blocks.find((block) => block.type === 'user')
      const stored = user?.type === 'user'
        ? user.content.find((block) => block.type === 'image')
        : undefined
      expect(stored && 'source' in stored ? stored.source : null).toEqual({
        type: 'ref',
        ref: attachment.sha256,
        mediaType: 'image/png'
      } as never)

      const blob = await world.backend.session.fetch(
        `/api/blobs/${attachment.sha256}?type=image/png`
      )
      expect(blob.status).toBe(200)
      expect(new Uint8Array(await blob.arrayBuffer())).toEqual(PNG_BYTES)
    }, 30_000)
  }
)
