import { z } from 'zod'
import { errorMessage } from '@demicodes/utils'
import type { AgentServer } from '@demicodes/agent'
import type { InstanceMode } from '../auth/identity'
import { conversationTargetSchema, type ControlService } from '../storage/control'
import type { ProviderVault } from '../vault/providers'
import { visibleProvider } from '../vault/scope'
import type { ConversationTargets } from './target'

export const conversationPatchSchema = z.strictObject({
  title: z.string().trim().min(1).max(256).optional(),
  archived: z.boolean().optional(),
  pinned: z.boolean().optional(),
  providerId: z.string().min(1).nullable().optional(),
  modelId: z.string().min(1).nullable().optional(),
  target: conversationTargetSchema.optional(),
})
export type ConversationPatch = z.infer<typeof conversationPatchSchema>
interface FieldResult { field: string; status: 'applied' | 'failed'; code?: string; message?: string; httpStatus?: 404 | 409 | 500 }

/** Applies independent fields with explicit outcomes. Archive and target changes share the existing admission gates. */
export class ConversationUpdates {
  constructor(private readonly deps: { control: ControlService; vault: ProviderVault; mode: InstanceMode; targets: ConversationTargets; agentServer: AgentServer }) {}

  async apply(userId: string, id: string, patch: ConversationPatch) {
    const { control } = this.deps
    const initial = await control.getConversation(id)
    if (!initial || initial.userId !== userId) return null
    const results: FieldResult[] = []
    const apply = async (field: string, operation: () => Promise<void | Omit<FieldResult, 'field'>>) => {
      try {
        const outcome = await operation()
        results.push({ field, ...outcome ?? { status: 'applied' } })
      } catch (error) {
        results.push({ field, status: 'failed', code: 'operation_failed', message: errorMessage(error), httpStatus: 500 })
      }
    }
    if (patch.archived !== undefined) await apply('archived', () => this.archive(id, patch.archived!))
    for (const field of ['title', 'pinned', 'model', 'target'] as const) {
      const present = field === 'model' ? patch.providerId !== undefined || patch.modelId !== undefined : patch[field] !== undefined
      if (!present) continue
      await apply(field, async () => {
        if (field === 'target') {
          const result = await this.deps.targets.switch(id, patch.target!)
          if (result.outcome === 'switched' || result.outcome === 'noop') return
          return refused(result.outcome, `Target switch refused: ${result.outcome}`, result.outcome.endsWith('_not_found') ? 404 : 409)
        }
        const release = this.deps.targets.files(id).tryEnter()
        if (!release) return refused('conversation_busy', 'A conversation operation is still running')
        try {
          const current = (await control.getConversation(id))!
          if (current.archived) return refused('archived', 'Restore the conversation before editing it')
          if (field === 'title') return await control.renameConversation(id, patch.title!)
          if (field === 'pinned') return await control.setConversationPinned(id, patch.pinned!)
          const providerId = patch.providerId === undefined ? current.providerId : patch.providerId
          const modelId = patch.modelId === undefined ? current.modelId : patch.modelId
          if (providerId && !(await visibleProvider(this.deps.vault, this.deps.mode, userId, providerId))) return refused('provider_not_found', 'No such provider', 404)
          if ((providerId === null) !== (modelId === null)) return refused('invalid_model', 'Provider and model must be selected or cleared together')
          return await control.setConversationModel(id, providerId, modelId)
        } finally {
          release()
        }
      })
    }
    return { conversation: await control.getConversation(id), results }
  }

  private async archive(id: string, archived: boolean): Promise<void | Omit<FieldResult, 'field'>> {
    const releaseTree = this.deps.agentServer.reserveTreeMutation(id)
    if (!releaseTree) return refused('turn_in_flight', 'A conversation with running work cannot be archived or restored')
    const releaseFiles = this.deps.targets.files(id).tryReserve()
    if (!releaseFiles) {
      releaseTree()
      return refused('turn_in_flight', 'A conversation operation is still running')
    }
    try {
      await this.deps.control.setConversationArchived(id, archived)
    } finally {
      releaseFiles()
      releaseTree()
    }
  }
}

function refused(code: string, message: string, httpStatus: 404 | 409 = 409): Omit<FieldResult, 'field'> {
  return { status: 'failed', code, message, httpStatus }
}
