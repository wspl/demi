import type { BlobStore } from '@demicodes/agent'
import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import type { ControlService } from '../storage/control'

/**
 * The upload's own media type, as `content-type` names it: one RFC 9110
 * `type/subtype`, with the parameters the sender attached. `multipart/*` is a
 * form envelope rather than a file's bytes, so it is not one of them.
 */
const attachmentMediaTypeSchema = z
  .string()
  .regex(
    /^[A-Za-z0-9!#$%&'*+.^_`|~-]+\/[A-Za-z0-9!#$%&'*+.^_`|~-]+\s*(;.*)?$/,
    'expected a type/subtype media type',
  )
  .refine(
    (value) => !value.toLowerCase().startsWith('multipart/'),
    'multipart uploads carry no single file',
  )

/**
 * Hardcoded upload ceiling (demi-next.md § Attachments: one number,
 * configurable later).
 */
export const ATTACHMENT_MAX_BYTES = 25 * 1024 * 1024

/**
 * `/api/attachments` — message-attachment upload: bytes into the blob store,
 * one metadata row, and the id goes back for the `send` frame's ref block.
 * Never inline media into the frame socket.
 */
export function attachmentRoutes(options: {
  control: ControlService;
  blobsFor: (userId: string) => BlobStore
}): Hono<AuthEnv> {
  const { control, blobsFor } = options
  const app = new Hono<AuthEnv>()

  app.post('/', async (c) => {
    const parsed = attachmentMediaTypeSchema.safeParse(
      c.req.header('content-type')
    )
    if (!parsed.success) {
      return c.json({
        code: 'invalid_body',
        message: 'Send the raw file bytes with its media type as content-type'
      }, 400)
    }
    const mediaType = parsed.data
    const bytes = new Uint8Array(await c.req.arrayBuffer())
    if (bytes.length === 0)
      return c.json({ code: 'invalid_body', message: 'Empty upload' }, 400)
    if (bytes.length > ATTACHMENT_MAX_BYTES) {
      return c.json({
        code: 'too_large',
        message: `Attachment exceeds the ${ATTACHMENT_MAX_BYTES}-byte limit`
      }, 413)
    }
    const sha256 = await blobsFor(c.get('user').id).put(bytes)
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
