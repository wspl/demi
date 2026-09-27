import type { ModelInfo } from '../transport/protocol'
import { THINKING_OFF } from './model-selection'

export interface ReasoningOption {
  label: string
  /** The effort the option chooses: one the model lists, or thinking off. */
  effort: string
}

export interface ReasoningState {
  /** The effort the model starts with, which an effort it does not offer shows. */
  defaultEffort: string
  options: ReasoningOption[]
  /** Whether thinking can be turned off. When false there is no Off option — the model
   *  (e.g. any Claude Code model) always thinks; you can only pick the effort. */
  canDisable: boolean
}

export function buildReasoningState(model: ModelInfo | null | undefined): ReasoningState | null {
  const reasoning = model?.reasoning
  if (!reasoning || reasoning.efforts.length === 0)
    return null
  const effortOptions = reasoning.efforts.map((effort): ReasoningOption => ({
    label: effortLabel(effort),
    effort,
  }))
  const options: ReasoningOption[] = reasoning.canDisable
    ? [{ label: 'Off', effort: THINKING_OFF }, ...effortOptions]
    : effortOptions
  return {
    defaultEffort: reasoning.defaultEffort ?? reasoning.efforts[0]!,
    options,
    canDisable: reasoning.canDisable,
  }
}

/** The option the effort `effort` shows: its own, else the model's default. */
export function reasoningOptionIndex(state: ReasoningState, effort: string | null): number {
  const selected = state.options.findIndex((option) => option.effort === effort)
  if (selected >= 0)
    return selected
  const fallback = state.options.findIndex((option) => option.effort === state.defaultEffort)
  return fallback >= 0 ? fallback : 0
}

export function reasoningOptionLabel(state: ReasoningState, effort: string | null): string {
  return state.options[reasoningOptionIndex(state, effort)]?.label ?? ''
}

function effortLabel(effort: string): string {
  if (effort.length === 0)
    return effort
  return effort.charAt(0).toUpperCase() + effort.slice(1)
}
