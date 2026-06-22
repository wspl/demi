import type { ThinkingConfig } from '@demi/core'
import type { ModelInfo } from '../transport/protocol'

export interface ReasoningOption {
  label: string
  config: ThinkingConfig
  group: 'general' | 'effort'
}

export interface ReasoningState {
  mode: 'toggle' | 'dropdown'
  defaultConfig: ThinkingConfig
  options: ReasoningOption[]
  /** Whether thinking can be turned off. When false there is no "No reasoning" option — the model
   *  (e.g. any Claude Code model) always thinks; you can only pick the effort level. */
  canDisable: boolean
}

export function buildReasoningState(model: ModelInfo | null | undefined): ReasoningState | null {
  const reasoning = model?.reasoning
  if (!reasoning || reasoning.efforts.length === 0) return null
  const defaultEffort = reasoning.defaultEffort ?? reasoning.efforts[0]!
  const effortOptions = reasoning.efforts.map((effort): ReasoningOption => ({
    label: effort.charAt(0).toUpperCase() + effort.slice(1),
    config: { type: 'effort', effort, summary: null },
    group: 'effort',
  }))
  const options: ReasoningOption[] = reasoning.canDisable
    ? [{ label: 'No reasoning', config: { type: 'disabled' }, group: 'general' }, ...effortOptions]
    : effortOptions
  return {
    mode: 'dropdown',
    defaultConfig: { type: 'effort', effort: defaultEffort, summary: null },
    options,
    canDisable: reasoning.canDisable,
  }
}

export function thinkingConfigToEffort(config: ThinkingConfig): string | null {
  return config.type === 'effort' || config.type === 'adaptive' ? config.effort : null
}

export function effortToThinkingConfig(effort: string | null): ThinkingConfig {
  return effort ? { type: 'effort', effort, summary: null } : { type: 'disabled' }
}
