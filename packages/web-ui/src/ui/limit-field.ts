import type { SentenceText } from './ui-text'

/**
 * A limit: the whole number from 1 last set, and whether it is lifted
 * (Unlimited). Lifting it keeps the number, so Limited brings it back.
 */
export interface LimitValue {
  limit: number
  unlimited: boolean
}

/**
 * What a limit field is doing besides showing its value (LimitField):
 * `shown`, at rest; `confirming` Unlimited in the dialog that warns first.
 */
export type LimitPhase = 'shown' | 'confirming'

/** What the user does to a limit field's choice of Limited or Unlimited, or to its dialog. */
export type LimitEvent = 'chooseLimited' | 'chooseUnlimited' | 'confirm' | 'cancel'

/** Where an event leaves the field, and the value it commits, if any. */
export interface LimitStep {
  phase: LimitPhase
  commit?: LimitValue
}

/**
 * The step `event` takes from `phase` while the value is `value`. Unlimited
 * is never committed without the dialog's confirmation; its Cancel, the
 * default answer, keeps the value. Either choice keeps the number: Limited
 * from Unlimited commits the number as it was, never an empty field.
 */
export function limitStep(value: LimitValue, phase: LimitPhase, event: LimitEvent): LimitStep {
  switch (event) {
    case 'chooseLimited':
      return value.unlimited ? { phase: 'shown', commit: { limit: value.limit, unlimited: false } } : { phase: 'shown' }
    case 'chooseUnlimited':
      return { phase: value.unlimited ? 'shown' : 'confirming' }
    case 'confirm':
      return phase === 'confirming' ? { phase: 'shown', commit: { limit: value.limit, unlimited: true } } : { phase }
    case 'cancel':
      return { phase: 'shown' }
  }
}

/** What a limit field says of text that is not a limit. */
export const LIMIT_PROBLEM: SentenceText = 'Enter a whole number from 1.'

/** The most a limit may be: the backend stores it as a 32-bit count. */
const LIMIT_MAX = 4_294_967_295

/** The limit `text` names, a whole number from 1, or null when it names none. */
export function parseLimit(text: string): number | null {
  const trimmed = text.trim()
  if (!/^\d+$/.test(trimmed)) {
    return null
  }
  const limit = Number(trimmed)
  return limit >= 1 && limit <= LIMIT_MAX ? limit : null
}
