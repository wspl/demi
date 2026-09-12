import { z } from 'zod'
import {
  createClientFrameSchema,
  userContentBlockSchema,
  displayedBlockSchema,
  displayedServerFrameSchema,
  type ClientFrame,
  type DisplayedBlock,
  type ServerFrame,
} from '@demicodes/agent/client'

export const safeFileNameSchema = z
  .string()
  .min(1)
  .max(255)
  .refine(
    (name) => !/[/\\\0]/.test(name) && name !== '.' && name !== '..',
    'Expected a file name without path separators',
  )
export const uploadRefBlockSchema = z.strictObject({
  type: z.literal('upload'),
  ref: z.string().min(1),
  fileName: safeFileNameSchema,
})
export const remoteFileRefSchema = z.strictObject({
  type: z.literal('remote_file'),
  deviceId: z.string().min(1),
  path: z
    .string()
    .min(1)
    .max(4096)
    .refine(
      (path) => path.startsWith('/') && !path.includes('\0'),
      'Expected an absolute path',
    ),
})
export const outgoingReferenceSchema = z.discriminatedUnion('type', [
  uploadRefBlockSchema,
  remoteFileRefSchema,
])
export type OutgoingReference = z.infer<typeof outgoingReferenceSchema>

const referencePrefix = 'demi-upload:'
const clientUserContentSchema = userContentBlockSchema
  .refine(
    (block) => block.type !== 'attachment',
    'An attachment record cannot be submitted in a send or steer',
  )
  .refine(
    (block) =>
      block.type !== 'reference' ||
      !block.reference.startsWith(referencePrefix),
    'Product references must use their upload or device wire shape',
  )
export const conversationClientFrameSchema = createClientFrameSchema(
  z.union([clientUserContentSchema, uploadRefBlockSchema, remoteFileRefSchema]),
).superRefine((frame, context) => {
  if (frame.type !== 'edit_and_send') {
    return
  }
  for (const [index, block] of frame.request.content.entries()) {
    if (
      block.type === 'reference' &&
      block.reference.startsWith(referencePrefix)
    ) {
      context.addIssue({
        code: 'custom',
        path: ['request', 'content', index],
        message:
          'Editing requires complete content; upload and device envelopes are not supported',
      })
    }
  }
})
export type ConversationClientFrame = z.infer<
  typeof conversationClientFrameSchema
>

/** Carries a product reference through the agent client's typed content API. */
export function contentReference(value: OutgoingReference) {
  return {
    type: 'reference' as const,
    reference:
      referencePrefix + JSON.stringify(outgoingReferenceSchema.parse(value)),
  }
}

/** Expands product references only where the conversation wire supports them. */
export function encodeClientFrame(frame: ClientFrame): ConversationClientFrame {
  if (frame.type !== 'send' && frame.type !== 'steer') {
    return conversationClientFrameSchema.parse(frame)
  }
  return conversationClientFrameSchema.parse({
    ...frame,
    content: frame.content.map((block) => {
      if (
        block.type !== 'reference' ||
        !block.reference.startsWith(referencePrefix)
      ) {
        return block
      }
      let value: unknown
      try {
        value = JSON.parse(block.reference.slice(referencePrefix.length))
      } catch {
        throw new Error('Invalid product reference JSON')
      }
      return outgoingReferenceSchema.parse(value)
    }),
  })
}

export const transcriptSchema = z.object({
  blocks: z.array(displayedBlockSchema),
  subagents: z.array(
    z.object({
      id: z.string(),
      name: z.string(),
      phase: z.enum(['running', 'completed', 'aborted', 'error']),
      startedAt: z.string(),
      endedAt: z.string().nullable(),
      blocks: z.array(displayedBlockSchema),
    }),
  ),
})
export type TranscriptResponse = z.infer<typeof transcriptSchema>

export function decodeServerFrame(value: unknown): ServerFrame<DisplayedBlock> {
  return displayedServerFrameSchema.parse(value)
}
