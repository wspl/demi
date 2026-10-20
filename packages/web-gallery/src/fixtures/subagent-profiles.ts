import { reactive } from 'vue'
import type { SettingsSubagentProfile } from '@demicodes/web-ui/settings/types'
import type { LimitValue } from '@demicodes/web-ui/ui/limit-field'

/**
 * A user's profiles, one of each kind the section shows:
 *
 * - On another provider's model with instructions of its own and no spawning.
 * - Inheriting the parent.
 * - Unavailable because its model no longer offers its effort.
 * - With a Context of its own.
 * - Disabled with its model gone as well.
 */
export function demoSubagentProfiles(): SettingsSubagentProfile[] {
  return [
    {
      id: 'archivist',
      name: 'archivist',
      description: 'Use to summarize old conversations into notes.',
      model: { providerId: 'openai', modelId: 'gpt-4', thinkingEffort: null, serviceTierId: null },
      contextLimit: null,
      instructions: null,
      canSpawn: true,
      enabled: false,
    },
    {
      id: 'explore',
      name: 'explore',
      description: 'Use for finding code, call sites and documentation. It changes nothing.',
      model: { providerId: 'openai', modelId: 'gpt-5', thinkingEffort: null, serviceTierId: 'priority' },
      contextLimit: null,
      instructions: 'Find code and report where it is, with paths and line numbers.\nChange nothing.',
      canSpawn: false,
      enabled: true,
    },
    {
      id: 'reviewer',
      name: 'reviewer',
      description: 'Use to review a finished change before reporting it.',
      model: null,
      contextLimit: null,
      instructions: null,
      canSpawn: true,
      enabled: true,
    },
    {
      id: 'scout',
      name: 'scout',
      description: 'Use for a quick look at one file.',
      model: { providerId: 'anthropic', modelId: 'claude-opus', thinkingEffort: 'minimal', serviceTierId: null },
      contextLimit: 500_000,
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
    maxLive: { limit: 10, unlimited: false } as LimitValue,
    maxDepth: { limit: 1, unlimited: false } as LimitValue,
    profiles: demoSubagentProfiles(),
  })
}

export type SubagentState = ReturnType<typeof createSubagentState>
