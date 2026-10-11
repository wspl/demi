import type { SentenceText } from './ui-text'

/**
 * What a limit field is doing besides showing its value (LimitField):
 * `shown`, at rest; `typing` a number after Limited was chosen while the
 * value is Unlimited, before one is committed; `confirming` Unlimited in
 * the dialog that warns first.
 */
export type LimitPhase = 'shown' | 'typing' | 'confirming'

/**
 * What the user does to a limit field; `leaveEmpty` leaves the field with no
 * text. Text that is no limit keeps the field as it is, with its problem.
 */
export type LimitEvent = 'chooseLimited' | 'chooseUnlimited' | 'confirm' | 'cancel' | 'leaveEmpty'

/** Where an event leaves the field, and the value it commits, if any: null for Unlimited. */
export interface LimitStep {
  phase: LimitPhase
  commit?: number | null
}

/**
 * The step `event` takes from `phase` while the value is `value`, null for
 * Unlimited. Unlimited is never committed without the dialog's confirmation;
 * its Cancel, the default answer, keeps the value. Limited from Unlimited
 * opens an empty field, and leaving it empty stays Unlimited.
 */
export function limitStep(value: number | null, phase: LimitPhase, event: LimitEvent): LimitStep {
  switch (event) {
    case 'chooseLimited':
      return { phase: value === null ? 'typing' : 'shown' }
    case 'chooseUnlimited':
      return { phase: value === null ? 'shown' : 'confirming' }
    case 'confirm':
      return phase === 'confirming' ? { phase: 'shown', commit: null } : { phase }
    case 'cancel':
      return { phase: phase === 'confirming' ? 'shown' : phase }
    case 'leaveEmpty':
      return { phase: phase === 'typing' ? 'shown' : phase }
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
