import { computed, reactive } from 'vue'
import type { ModelInfo } from '@demicodes/web-ui/transport/protocol'
import type { SmallModelChoice } from '@demicodes/web-ui/settings/types'
import { composerModels } from './context-limits'

/** The state of Settings › Agent's Small Model group in a specimen. */
export interface SmallModelState {
  choice: SmallModelChoice | null
  sameProvider: boolean
}

export function createSmallModelState(choice: SmallModelChoice | null = null, sameProvider = false): SmallModelState {
  return reactive({ choice, sameProvider })
}

/** Claude Haiku, which the gallery's Anthropic entry lists beside the composer's models for the small model's menu. */
const haiku: ModelInfo = {
  id: 'claude-haiku-4-5',
  name: 'Claude Haiku 4.5',
  contextWindow: 200_000,
  contextLimit: null,
  acceptedExtensions: ['png', 'pdf', 'md'],
  reasoning: { efforts: ['low', 'medium', 'high'], canDisable: true },
  serviceTiers: null,
}

/** The composer's models with Claude Haiku, as an entry that offers a small model lists them. */
export const smallModelCatalog = computed<Record<string, ModelInfo[]>>(() => ({
  ...composerModels.value,
  anthropic: [...(composerModels.value.anthropic ?? []), haiku],
}))

/**
 * What the backend answers Automatic resolves to for these entries
 * (`models.md` § Small model): Claude Haiku, or nothing while small
 * requests stay on the conversation's provider, which the page cannot know.
 */
export function galleryAutomaticModel(state: SmallModelState): string | null {
  return state.sameProvider ? null : haiku.name
}
