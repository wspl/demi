import type { FileExtension, ModelSelection, ThinkingCapability, ThinkingSummary } from '@demi/core'
import type { ProviderModel } from '@demi/provider'
import type { ModelInfo } from '@demi/web-ui/transport/protocol'

const ATTACHMENT_EXTENSIONS: FileExtension[] = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'pdf']

export function toModelInfo(model: ProviderModel): ModelInfo {
  return {
    id: model.id,
    name: model.displayName,
    contextWindow: model.contextWindow,
    inputLimit: null,
    acceptedExtensions: model.supportsAttachments ? [...ATTACHMENT_EXTENSIONS] : [],
    reasoning:
      model.supportedThinkingEfforts && model.supportedThinkingEfforts.length > 0
        ? { efforts: [...model.supportedThinkingEfforts], defaultEffort: model.defaultThinkingEffort ?? null }
        : null,
  }
}

export function buildModelSelection(
  providerId: string,
  modelId: string,
  thinkingEffort: string | null,
  serviceTierId: string | null,
  model: ProviderModel | null,
): ModelSelection {
  return {
    providerId,
    model: {
      id: modelId,
      name: model?.displayName ?? `${providerId} ${modelId}`,
      contextWindow: model?.contextWindow ?? 0,
      inputLimit: null,
      thinking: thinkingCapabilities(model),
      acceptedExtensions: model?.supportsAttachments ? [...ATTACHMENT_EXTENSIONS] : [],
    },
    thinking: thinkingEffort ? { type: 'effort', effort: thinkingEffort, summary: null } : null,
    serviceTierId,
  }
}

function thinkingCapabilities(model: ProviderModel | null): ThinkingCapability[] {
  if (!model) return []
  if (model.supportsReasoning === false) return [{ type: 'disabled' }]
  if (!model.supportedThinkingEfforts || model.supportedThinkingEfforts.length === 0) return []
  const summaries: ThinkingSummary[] = ['auto', 'concise', 'detailed', 'off', 'on']
  return [
    {
      type: 'effort',
      efforts: [...model.supportedThinkingEfforts],
      defaultEffort: model.defaultThinkingEffort ?? null,
      summaries,
      defaultSummary: null,
    },
  ]
}
