import { z } from 'zod'

export const conversationTargetSchema = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('cloud'),
      path: z.string().startsWith('/').optional(),
    })
    .strict(),
  z
    .object({
      kind: z.literal('device'),
      deviceId: z.string().min(1),
      path: z.string().min(1),
    })
    .strict(),
  z
    .object({
      kind: z.literal('workspace'),
      workspaceId: z.string().min(1),
    })
    .strict(),
])
export type ConversationTargetPointer = z.infer<typeof conversationTargetSchema>

export const targetSchema = conversationTargetSchema
