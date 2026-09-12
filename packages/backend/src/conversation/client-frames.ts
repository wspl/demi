import {
  clientFrameSchema,
  sendFrameSchema,
  steerFrameSchema,
  userContentBlockSchema
} from '@demicodes/agent'
import { z } from 'zod'
import { remoteFileRefSchema } from './remote-file-refs'
import { uploadRefBlockSchema } from './attachment-refs'

/**
 * The conversation wire adds upload and remote-file references to
 * content-bearing frames. An `attachment` block is the backend's own record
 * of a resolved upload; a client never sends one.
 */
const clientUserContentSchema = userContentBlockSchema.refine(
  (block) => block.type !== 'attachment',
  'An attachment block is written by the backend, not sent by a client',
)
const content = z.array(z.union([
  clientUserContentSchema,
  uploadRefBlockSchema,
  remoteFileRefSchema
]))
export const conversationClientFrameSchema = z.union([
  clientFrameSchema,
  sendFrameSchema.extend({ content }),
  steerFrameSchema.extend({ content }),
])
export type ConversationClientFrame = z.infer<typeof conversationClientFrameSchema>
