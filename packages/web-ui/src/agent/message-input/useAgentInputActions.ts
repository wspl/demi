import type { ThinkingConfig, UserContentBlock } from '@demi/core'
import { reportError } from '@demi/web-ui/infra/errors'
import type { AgentWorkspace } from '../workspace'
import { thinkingConfigToEffort } from '../reasoning'
import { queuedMessageIdForEmptySubmit } from '../queue-submit'

interface UseAgentInputActionsParams {
  workspace: AgentWorkspace
  conversationId: string
  buildSubmitPayload: () => UserContentBlock[] | null
  clearInput: () => void
  emitEmptySubmit?: () => void
}

export function useAgentInputActions(params: UseAgentInputActionsParams) {
  async function handleSubmit(): Promise<void> {
    const content = params.buildSubmitPayload()
    if (!content) {
      const session = params.workspace.sessions[params.conversationId]
      const queue = session?.queue ?? []
      const messageId = queuedMessageIdForEmptySubmit(queue)
      if (messageId) {
        if (session?.phase === 'running') {
          try {
            await params.workspace.steerQueuedMessage(params.conversationId, messageId)
          } catch (error) {
            reportError('Failed to steer queued message', error, { userVisible: true })
          }
        } else {
          params.workspace.sendQueuedMessage(params.conversationId, messageId)
        }
        return
      }
      params.emitEmptySubmit?.()
      return
    }
    params.clearInput()
    try {
      await params.workspace.send(params.conversationId, content)
    } catch (error) {
      reportError('Failed to send message', error, { userVisible: true })
    }
  }

  async function handleSteerSubmit(): Promise<void> {
    const content = params.buildSubmitPayload()
    if (!content) return
    params.clearInput()
    try {
      await params.workspace.steer(params.conversationId, content)
    } catch (error) {
      reportError('Failed to steer turn', error, { userVisible: true })
    }
  }

  async function handleQueueSubmit(): Promise<void> {
    const content = params.buildSubmitPayload()
    if (!content) return
    params.clearInput()
    try {
      await params.workspace.send(params.conversationId, content)
    } catch (error) {
      reportError('Failed to queue message', error, { userVisible: true })
    }
  }

  function handleSelectModel(providerType: string, modelId: string): void {
    const current = params.workspace.sessions[params.conversationId]?.model
    params.workspace.setModel(params.conversationId, {
      providerType,
      modelId,
      thinkingEffort: current?.thinkingEffort ?? null,
      serviceTierId: null,
    })
  }

  function handleChangeThinking(config: ThinkingConfig): void {
    const current = params.workspace.sessions[params.conversationId]?.model
    if (!current) return
    params.workspace.setModel(params.conversationId, { ...current, thinkingEffort: thinkingConfigToEffort(config) })
  }

  function handleAbort(): void {
    void params.workspace.abort(params.conversationId).catch((error) => {
      reportError('Failed to abort conversation', error, { userVisible: true })
    })
  }

  function handleCompact(): void {
    void params.workspace.compact(params.conversationId).catch(() => {
      // phase stream carries failure/abort status
    })
  }

  return { handleSubmit, handleSteerSubmit, handleQueueSubmit, handleSelectModel, handleChangeThinking, handleAbort, handleCompact }
}
