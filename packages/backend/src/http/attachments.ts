import type { BlobStore } from '@demicodes/agent'
import { Hono } from 'hono'
import type { AuthEnv } from '../auth/identity'
import type { ControlService } from '../storage/control'

/** Hardcoded upload ceiling (demi-next.md § Attachments: one number, configurable later). */
export const ATTACHMENT_MAX_BYTES = 25 * 1024 * 1024

/**
 * `/api/attachments` — message-attachment upload: bytes into the blob store,
 * one metadata row, and the id goes back for the `send` frame's ref block.
 * Never inline media into the frame socket.
 */
export function attachmentRoutes(options: { control: ControlService; blobs: BlobStore }): Hono<AuthEnv> {
  const { control, blobs } = options
  const app = new Hono<AuthEnv>()

  app.post('/', async (c) => {
    const mediaType = c.req.header('content-type')
    if (!mediaType || mediaType.startsWith('multipart/')) {
      return c.json({ code: 'invalid_body', message: 'Send the raw file bytes with its media type as content-type' }, 400)
    }
    const bytes = new Uint8Array(await c.req.arrayBuffer())
    if (bytes.length === 0) return c.json({ code: 'invalid_body', message: 'Empty upload' }, 400)
    if (bytes.length > ATTACHMENT_MAX_BYTES) {
      return c.json({ code: 'too_large', message: `Attachment exceeds the ${ATTACHMENT_MAX_BYTES}-byte limit` }, 413)
    }
    const sha256 = await blobs.put(bytes)
    const attachment = await control.createAttachment({
      userId: c.get('user').id,
      mediaType,
      sizeBytes: bytes.length,
      sha256,
    })
    return c.json({ attachment }, 201)
  })

  return app
}
