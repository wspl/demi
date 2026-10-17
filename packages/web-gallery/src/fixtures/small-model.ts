import { computed, reactive } from 'vue'
import type { ModelInfo } from '@demicodes/web-ui/transport/protocol'
import type { ImageEntryChoices, ImageVendorId, SettingsProviderEntry, SmallModelChoice } from '@demicodes/web-ui/settings/types'
import { composerModels } from './context-limits'

/** The state of Settings › Agent's Small Model and Image Generation groups in a specimen. */
export interface SmallModelState {
  choice: SmallModelChoice | null
  sameProvider: boolean
  /** The entry chosen for each vendor's images; a vendor absent is Automatic. */
  imageEntries: ImageEntryChoices
}

export function createSmallModelState(choice: SmallModelChoice | null = null, sameProvider = false): SmallModelState {
  return reactive({ choice, sameProvider, imageEntries: {} })
}

/**
 * What the backend answers each vendor's image Automatic takes for these
 * entries (`images.md` § Which entry makes an image): the first whose Image
 * Generation is on and that is ready, the subscriptions before the keys,
 * or null for a vendor whose entries cannot.
 */
export function galleryAutomaticImageEntries(
  entries: readonly SettingsProviderEntry[],
): Partial<Record<ImageVendorId, string | null>> {
  const automatic: Partial<Record<ImageVendorId, string | null>> = {}
  const ordered = [...entries].sort((a, b) => Number(a.kind !== 'subscription') - Number(b.kind !== 'subscription'))
  for (const entry of ordered) {
    const vendor = entry.imageVendor
    if (!vendor || !entry.enabled) {
      continue
    }
    if (!automatic[vendor] && entry.imageGeneration && entry.state === 'ready') {
      automatic[vendor] = entry.id
    } else {
      automatic[vendor] ??= null
    }
  }
  return automatic
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
