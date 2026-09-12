import type { AgentMessage } from '@demicodes/core'
import { z } from 'zod'
import { completionMessageId } from '../store/tree-store'
import { metadataSchema, modelSelectionSchema } from './schemas'
import type { PendingInternalSteer } from '../session/steer-queue'

export const agentMessageSchema: z.ZodType<AgentMessage> = z.object({
  id: z.string().min(1),
  sender: z.object({
    id: z.string().min(1),
    description: z.string(),
    round: z.number().int().nonnegative(),
  }).strict(),
  recipientId: z.string().min(1),
  timestamp: z.string().datetime(),
  content: z.string(),
  event: z.discriminatedUnion('type', [
    z.object({ type: z.literal('message') }).strict(),
    z.object({
      type: z.literal('completion'),
      outcome: z.enum(['completed', 'failed', 'aborted']),
    }).strict(),
  ]),
}).strict().refine(
  message => message.event.type === 'completion' || message.content.trim().length > 0,
  'Explicit agent messages must not be empty',
).refine(
  message => message.event.type !== 'completion'
    || message.id === completionMessageId(message.sender.id, message.sender.round),
  'Completion id must identify its source round',
)

export const pendingInternalSteerSchema: z.ZodType<PendingInternalSteer> = z.object({
  turnId: z.string().min(1),
  model: modelSelectionSchema,
  agentMessage: agentMessageSchema,
  metadata: metadataSchema.nullable(),
}).strict()

