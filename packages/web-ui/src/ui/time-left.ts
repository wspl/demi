import { t } from './locales/text'

/**
 * The time a task has left, as macOS Software Update says it: an estimate
 * while `seconds` is unknown, then under a minute, then about so many minutes.
 */
export function formatTimeLeft(seconds: number | null): string {
  if (seconds === null)
    return t('updating.estimating')
  if (seconds < 60)
    return t('updating.underAMinute')
  const minutes = Math.round(seconds / 60)
  return t('updating.minutesLeft', { count: minutes }, minutes)
}
