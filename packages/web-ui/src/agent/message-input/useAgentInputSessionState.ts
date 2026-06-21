import { computed } from 'vue'
import type { ThinkingConfig, TokenUsage } from '@demi/core'
import type { AgentWorkspace } from '../workspace'
import { getLatestResponseUsage } from '../block-helpers'
import { effortToThinkingConfig } from '../reasoning'

export function useAgentInputSessionState(workspace: AgentWorkspace, conversationId: string) {
  const session = computed(() => workspace.sessions[conversationId])

  const selectedProviderType = computed<string | null>(() => session.value?.model.providerType ?? null)
  const selectedModelId = computed<string | null>(() => session.value?.model.modelId ?? null)

  const selectedModel = computed(() => {
    const providerType = selectedProviderType.value
    const modelId = selectedModelId.value
    if (!providerType || !modelId) return null
    return (workspace.models[providerType] ?? []).find((model) => model.id === modelId) ?? null
  })

  const thinkingConfig = computed<ThinkingConfig>(() => effortToThinkingConfig(session.value?.model.thinkingEffort ?? null))
  const contextWindow = computed<number | null>(() => selectedModel.value?.contextWindow ?? null)
  const inputLimit = computed<number | null>(() => selectedModel.value?.inputLimit ?? null)
  const acceptedExtensions = computed<string[]>(() => selectedModel.value?.acceptedExtensions ?? [])

  const phase = computed(() => session.value?.phase ?? 'idle')
  const isRunning = computed(() => phase.value === 'running')
  const isCompacting = computed(() => phase.value === 'compacting')
  const canCompact = computed(() => (session.value?.blocks.length ?? 0) > 0)

  const usage = computed<TokenUsage | null>(() => {
    const blocks = session.value?.blocks
    return blocks ? getLatestResponseUsage(blocks) : null
  })

  return {
    selectedProviderType,
    selectedModelId,
    selectedModel,
    thinkingConfig,
    contextWindow,
    inputLimit,
    acceptedExtensions,
    isRunning,
    isCompacting,
    canCompact,
    usage,
  }
}
