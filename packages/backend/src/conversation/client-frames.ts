import { clientFrameSchema, sendFrameSchema, steerFrameSchema, userContentBlockSchema } from '@demicodes/agent'
import { z } from 'zod'
import { attachmentRefBlockSchema } from './attachment-refs'

/** The conversation wire adds uploaded attachment references to content-bearing frames. */
const content = z.array(z.union([userContentBlockSchema, attachmentRefBlockSchema]))
export const conversationClientFrameSchema = z.union([
  clientFrameSchema,
  sendFrameSchema.extend({ content }),
  steerFrameSchema.extend({ content }),
])
export type ConversationClientFrame = z.infer<typeof conversationClientFrameSchema>
