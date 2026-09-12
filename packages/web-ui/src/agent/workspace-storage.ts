import { z } from 'zod'
import { modelIntentSchema } from '../transport/protocol'

export const persistedWorkspaceSchema = z
  .strictObject({
    activeId: z.string().min(1).nullable(),
    conversations: z.array(
      z.strictObject({
        id: z.string().min(1),
        title: z.string(),
        createdAt: z.iso.datetime({ offset: true }),
        model: modelIntentSchema,
      }),
    ),
  })
  .superRefine((workspace, context) => {
    const ids = new Set(
      workspace.conversations.map((conversation) => conversation.id),
    )
    if (
      ids.size !== workspace.conversations.length ||
      (workspace.activeId !== null && !ids.has(workspace.activeId))
    ) {
      context.addIssue({
        code: 'custom',
        message: 'Invalid workspace conversation references',
      })
    }
  })
export type PersistedWorkspace = z.infer<typeof persistedWorkspaceSchema>

/** A corrupt list remains available for recovery; it never becomes a fresh list. */
export function readWorkspace(
  storage: Pick<Storage, 'getItem'>,
  key: string,
): PersistedWorkspace | null {
  const raw = storage.getItem(key)
  if (raw === null) {
    return null
  }
  try {
    return persistedWorkspaceSchema.parse(JSON.parse(raw))
  } catch {
    throw new Error('Invalid saved workspace')
  }
}
