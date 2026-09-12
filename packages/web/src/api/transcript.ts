import { z } from 'zod'
import {
  displayedBlockSchema as blockSchema,
  displayedServerFrameSchema,
  type ServerFrame,
  type ClientFrame,
} from '@demicodes/web-ui/transport/protocol'
export { displayedUserContentSchema, usageSchema, shellStatusSchema } from '@demicodes/web-ui/transport/protocol'
export { blockSchema }

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

export function decodeServerFrame(value: unknown): ServerFrame {
  return displayedServerFrameSchema.parse(value)
}

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
