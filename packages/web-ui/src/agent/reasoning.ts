import { upperFirst } from '@demicodes/utils'
import type { ModelInfo } from '../transport/protocol'
import { firstEffort, THINKING_OFF } from './model-selection'
import type { TitleText } from '../ui/ui-text'

export interface ReasoningOption {
  label: TitleText
  /** The effort the option chooses: one the model lists, or thinking off. */
  effort: string
}

export interface ReasoningState {
  /**
   * Off, when the model can turn thinking off, and the efforts the model
   * lists: only choices whose effect the user can tell
   * (`models.md` § A conversation's model settings).
   */
  options: ReasoningOption[]
  /** The effort settings that name none hold on this model: its first. */
  firstEffort: string
}

export function buildReasoningState(model: ModelInfo | null | undefined): ReasoningState | null {
  const first = model ? firstEffort(model) : null
  if (!model?.reasoning || first === null)
    return null
  const effortOptions = model.reasoning.efforts.map((effort): ReasoningOption => ({
    label: upperFirst(effort),
    effort,
  }))
  const options: ReasoningOption[] = model.reasoning.canDisable
    ? [{ label: 'Off', effort: THINKING_OFF }, ...effortOptions]
    : effortOptions
  return { options, firstEffort: first }
}

/**
 * The option the effort `effort` shows: its own; else, for none or one the
 * model does not offer, the option of the model's first effort, which a send
 * turns it into.
 */
export function reasoningOptionIndex(state: ReasoningState, effort: string | null): number {
  const selected = state.options.findIndex((option) => option.effort === effort)
  if (selected >= 0)
    return selected
  return state.options.findIndex((option) => option.effort === state.firstEffort)
}

export function reasoningOptionLabel(state: ReasoningState, effort: string | null): string {
  return state.options[reasoningOptionIndex(state, effort)]?.label ?? ''
}

