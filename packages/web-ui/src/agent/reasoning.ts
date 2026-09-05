import type { ThinkingConfig } from '@demicodes/core'
import { clamp } from '@demicodes/utils'
import type { ModelInfo } from '../transport/protocol'

export interface ReasoningOption {
  label: string
  config: ThinkingConfig
}

export interface ReasoningState {
  defaultConfig: ThinkingConfig
  options: ReasoningOption[]
  /** Whether thinking can be turned off. When false there is no Off option — the model
   *  (e.g. any Claude Code model) always thinks; you can only pick the effort. */
  canDisable: boolean
}

export function buildReasoningState(model: ModelInfo | null | undefined): ReasoningState | null {
  const reasoning = model?.reasoning
  if (!reasoning || reasoning.efforts.length === 0) return null
  const defaultEffort = reasoning.defaultEffort ?? reasoning.efforts[0]!
  const effortOptions = reasoning.efforts.map((effort): ReasoningOption => ({
    label: effortLabel(effort),
    config: { type: 'effort', effort, summary: null },
  }))
  const options: ReasoningOption[] = reasoning.canDisable
    ? [{ label: 'Off', config: { type: 'disabled' } }, ...effortOptions]
    : effortOptions
  return {
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

export function resolveThinkingConfig(state: ReasoningState, config: ThinkingConfig | undefined): ThinkingConfig {
  const cfg = config ?? state.defaultConfig
  if (!state.canDisable && cfg.type === 'disabled') return state.defaultConfig
  return cfg
}

function configsMatch(left: ThinkingConfig, right: ThinkingConfig): boolean {
  if (left.type !== right.type) return false
  if (left.type === 'adaptive' && right.type === 'adaptive') return left.effort === right.effort
  if (left.type === 'effort' && right.type === 'effort') return left.effort === right.effort
  return true
}

export function reasoningOptionIndex(state: ReasoningState, config: ThinkingConfig | undefined): number {
  const resolved = resolveThinkingConfig(state, config)
  const selected = state.options.findIndex((option) => configsMatch(option.config, resolved))
  if (selected >= 0) return selected
  const fallback = state.options.findIndex((option) => configsMatch(option.config, state.defaultConfig))
  return fallback >= 0 ? fallback : 0
}

export function reasoningOptionConfig(state: ReasoningState, index: number): ThinkingConfig {
  const max = state.options.length - 1
  const clamped = clamp(Math.round(index), 0, max)
  return state.options[clamped]?.config ?? state.defaultConfig
}

export function reasoningOptionLabel(state: ReasoningState, config: ThinkingConfig | undefined): string {
  return state.options[reasoningOptionIndex(state, config)]?.label ?? ''
}

function effortLabel(effort: string): string {
  if (effort.length === 0) return effort
  return effort.charAt(0).toUpperCase() + effort.slice(1)
}
