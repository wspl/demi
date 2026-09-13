import { z } from 'zod'
import {
  blockSchema,
  refSourceSchema,
  type ClientFrame,
} from '@demicodes/web-ui/transport/protocol'

const text = z.object({
  type: z.literal('text'),
  text: z.string(),
})
const urlSource = z.object({
  type: z.literal('url'),
  url: z.string(),
})
const binarySource = z.object({
  type: z.literal('binary'),
  data: z.instanceof(Uint8Array),
  mediaType: z.string(),
})
export const displayedUserContentSchema = z.discriminatedUnion('type', [
  text,
  z.object({
    type: z.literal('reference'),
    reference: z.string(),
  }),
  z.object({
    type: z.literal('attachment'),
    name: z.string(),
    path: z.string(),
    mediaType: z.string(),
    sizeBytes: z.number(),
    sha256: z.string(),
    snippet: z.string().optional(),
  }),
  z.object({
    type: z.literal('image'),
    source: z.union([refSourceSchema, urlSource, binarySource]),
  }),
  z.object({
    type: z.literal('video'),
    source: z.union([refSourceSchema, urlSource, binarySource]),
  }),
  z.object({
    type: z.literal('document'),
    source: z.union([
      refSourceSchema,
      z.object({
        data: z.instanceof(Uint8Array),
        mediaType: z.string(),
        fileName: z.string(),
      }),
    ]),
  }),
])
export const transcriptSchema = z.object({
  blocks: z.array(blockSchema),
  subagents: z.array(
    z.object({
      id: z.string(),
      name: z.string(),
      phase: z.enum(['running', 'completed', 'aborted', 'error']),
      startedAt: z.string(),
      endedAt: z.string().nullable(),
      blocks: z.array(blockSchema),
    }),
  ),
})
const outgoingReferenceSchema = z.discriminatedUnion('type', [
  z.object({
    type: z.literal('remote_file'),
    deviceId: z.string(),
    path: z.string(),
  }),
  z.object({
    type: z.literal('upload'),
    ref: z.string(),
    fileName: z.string(),
  }),
])
export type OutgoingReference = z.infer<typeof outgoingReferenceSchema>
const referencePrefix = 'demi-upload:'
export function contentReference(value: OutgoingReference) {
  return {
    type: 'reference' as const,
    reference: referencePrefix + JSON.stringify(value),
  }
}

/** Keep the runtime's core content contract while adapting product wire refs. */
export function encodeClientFrame(frame: ClientFrame): unknown {
  if (!('content' in frame) || !Array.isArray(frame.content)) {
    return frame
  }
  return {
    ...frame,
    content: frame.content.map((block) =>
      block.type === 'reference' && block.reference.startsWith(referencePrefix)
        ? outgoingReferenceSchema.parse(
            JSON.parse(block.reference.slice(referencePrefix.length)),
          )
        : block,
    ),
  }
}
