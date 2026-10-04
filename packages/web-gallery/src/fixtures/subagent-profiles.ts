import { reactive } from 'vue'
import type { SettingsSubagentProfile } from '@demicodes/web-ui/settings/types'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'
import { demoModels, demoProviders } from './catalog'

/** The catalog the Subagent section's model menu offers and its marks are read from. */
export const subagentProviders: ProviderInfo[] = demoProviders
export const subagentModels: Record<string, ModelInfo[]> = demoModels

/**
 * A user's profiles, one of each kind the section shows: on another
 * provider's model with its own prompt and no spawning, inheriting the
 * parent, unavailable because its model no longer offers its effort, and
 * disabled with its model gone as well.
 */
export function demoSubagentProfiles(): SettingsSubagentProfile[] {
  return [
    {
      id: 'archivist',
      name: 'archivist',
      description: 'Use to summarize old conversations into notes.',
      model: { providerId: 'openai', modelId: 'gpt-4', thinkingEffort: null, serviceTierId: null },
      instructions: null,
      canSpawn: true,
      enabled: false,
    },
    {
      id: 'explore',
      name: 'explore',
      description: 'Use for finding code, call sites and documentation; it reports and changes nothing.',
      model: { providerId: 'openai', modelId: 'gpt-5', thinkingEffort: null, serviceTierId: 'priority' },
      instructions: 'You find code and report where it is, with paths and line numbers.\nYou change nothing.',
      canSpawn: false,
      enabled: true,
    },
    {
      id: 'reviewer',
      name: 'reviewer',
      description: 'Use to review a finished change before reporting it.',
      model: null,
      instructions: null,
      canSpawn: true,
      enabled: true,
    },
    {
      id: 'scout',
      name: 'scout',
      description: 'Use for a quick look at one file.',
      model: { providerId: 'anthropic', modelId: 'claude-opus', thinkingEffort: 'minimal', serviceTierId: null },
      instructions: null,
      canSpawn: false,
      enabled: true,
    },
  ]
}

/** The Subagent section's state as a specimen holds it. */
export function createSubagentState(enabled = true) {
  return reactive({
    enabled,
    profiles: demoSubagentProfiles(),
  })
}

export type SubagentState = ReturnType<typeof createSubagentState>
