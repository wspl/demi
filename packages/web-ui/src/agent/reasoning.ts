import type { ModelInfo } from '../transport/protocol'
import { THINKING_OFF } from './model-selection'

export interface ReasoningOption {
  label: string
  /**
   * The effort the option chooses: one the model lists, thinking off, or
   * null, the model's default, which sends no thinking setting.
   */
  effort: string | null
}

export interface ReasoningState {
  /**
   * The effort settings that name none hold on this model, as the backend
   * says: null, the Default option, when the model can turn thinking off
   * (`models.md` § A conversation's model settings).
   */
  unnamedEffort: string | null
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
  // A model that can turn thinking off also has a default of its own, which
  // the request leaves to the vendor; one that cannot always names an effort.
  const options: ReasoningOption[] = reasoning.canDisable
    ? [{ label: 'Default', effort: null }, { label: 'Off', effort: THINKING_OFF }, ...effortOptions]
    : effortOptions
  return {
    unnamedEffort: reasoning.unnamedEffort,
    options,
    canDisable: reasoning.canDisable,
  }
}

/**
 * The option the effort `effort` shows: its own; else, for none or one the
 * model does not offer, which a send turns into none, the option of the
 * effort that none holds.
 */
export function reasoningOptionIndex(state: ReasoningState, effort: string | null): number {
  const selected = state.options.findIndex((option) => option.effort === effort)
  if (selected >= 0)
    return selected
  const unnamed = state.options.findIndex((option) => option.effort === state.unnamedEffort)
  return unnamed >= 0 ? unnamed : 0
}

export function reasoningOptionLabel(state: ReasoningState, effort: string | null): string {
  return state.options[reasoningOptionIndex(state, effort)]?.label ?? ''
}

function effortLabel(effort: string): string {
  if (effort.length === 0)
    return effort
  return effort.charAt(0).toUpperCase() + effort.slice(1)
}
