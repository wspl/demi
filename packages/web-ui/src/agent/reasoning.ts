import type { Component } from 'vue'
import { Battery, BatteryCharging, BatteryFull, BatteryLow, BatteryMedium, Gauge } from '@lucide/vue'
import { upperFirst } from '@demicodes/utils'
import { find, t } from './locales/text'
import type { ModelInfo } from '../transport/protocol'
import { firstEffort, THINKING_OFF } from './model-selection'
import type { TitleText } from '../ui/ui-text'

export interface ReasoningOption {
  label: TitleText
  /** The effort the option chooses: one the model lists, or thinking off. */
  effort: string
  /** A charge that rises with the effort, as a level meter does. */
  icon: Component
}

const EFFORT_ICONS: Record<string, Component> = {
  [THINKING_OFF]: Battery,
  minimal: BatteryLow,
  low: BatteryLow,
  medium: BatteryMedium,
  high: BatteryFull,
  xhigh: BatteryCharging,
  max: BatteryCharging,
}

export interface ReasoningState {
  /**
   * Off, when the model can turn thinking off, and the efforts the model
   * lists: only choices whose effect the user can tell
   * (`packages/web-ui/src/agent/model-selection.ts`).
   */
  options: ReasoningOption[]
  /** The effort settings that name none hold on this model: its first. */
  firstEffort: string
}

/**
 * The charge of an effort, or of thinking off; an effort the table does not
 * name reads as a plain gauge.
 */
export function effortIcon(effort: string): Component {
  return EFFORT_ICONS[effort] ?? Gauge
}

/** An effort as the model menu names it: `high` is High. */
export function effortWord(effort: string): TitleText {
  return find(`models.effort.${effort}`) ?? upperFirst(effort)
}

export function buildReasoningState(model: ModelInfo | null | undefined): ReasoningState | null {
  const first = model ? firstEffort(model) : null
  if (!model?.reasoning || first === null)
    return null
  const effortOptions = model.reasoning.efforts.map((effort): ReasoningOption => ({
    label: effortWord(effort),
    effort,
    icon: effortIcon(effort),
  }))
  const options: ReasoningOption[] = model.reasoning.canDisable
    ? [{ label: t('models.off'), effort: THINKING_OFF, icon: effortIcon(THINKING_OFF) }, ...effortOptions]
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

