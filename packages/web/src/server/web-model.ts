import {
  VIDEO_FILE_EXTENSIONS,
  type FileExtension,
  type ModelSelection,
  type ThinkingCapability,
  type ThinkingSummary,
} from '@demicodes/core'
import { DEFAULT_ATTACHMENT_EXTENSIONS, type ProviderModel } from '@demicodes/provider'
import type { ModelInfo } from '@demicodes/web-ui/transport/protocol'

function acceptedExtensionsFor(model: ProviderModel | null): FileExtension[] {
  return [
    ...(model?.supportsAttachments ? DEFAULT_ATTACHMENT_EXTENSIONS : []),
    ...(model?.supportsVideo ? VIDEO_FILE_EXTENSIONS : []),
  ]
}

export function toModelInfo(model: ProviderModel): ModelInfo {
  return {
    id: model.id,
    name: model.displayName,
    contextWindow: model.contextWindow,
    inputLimit: null,
    acceptedExtensions: acceptedExtensionsFor(model),
    reasoning:
      model.supportedThinkingEfforts && model.supportedThinkingEfforts.length > 0
        ? {
            efforts: [...model.supportedThinkingEfforts],
            defaultEffort: model.defaultThinkingEffort ?? null,
            canDisable: model.canDisableThinking ?? true,
          }
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
      acceptedExtensions: acceptedExtensionsFor(model),
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
