import type { QuotaWindowName } from '@demicodes/protocol'
import { pageLanguage } from '../i18n/text'
import { t } from './locales/text'

/**
 * A usage window's name in the page's language, from its kind, its period and its
 * scope, the same for every vendor (`packages/web-ui/src/ui/dates.ts`): *5-hour
 * limit*, *Weekly Opus limit*, *Monthly credits*, *Usage limit*.
 *
 * Settings, the context card and the account switch's line all name a window through
 * this.
 */
export function windowName(name: QuotaWindowName): string {
  return named(name, (scope) => scope)
}

/**
 * The window's name inside a sentence, *reached its weekly Opus limit*.
 *
 * - In a language with capital letters it is lowercase but for the model it is
 *   on, a proper name.
 * - In Chinese and Japanese, which put no spaces between words, it has a space on
 *   a side that is a Latin letter or a digit, so it sits by the Chinese or
 *   Japanese text around it as the Writing page asks (*已达到 5 小时限额*).
 */
export function windowNameInSentence(name: QuotaWindowName): string {
  const script = new Intl.Locale(pageLanguage()).maximize().script
  if (script === 'Latn') {
    // The scope is filled in as it is after the rest is lowercased.
    const marker = '\u0000'
    return named(name, () => marker).toLocaleLowerCase(pageLanguage()).replace(marker, name.scope?.label ?? '')
  }
  const text = windowName(name)
  if (script === 'Hans' || script === 'Hant' || script === 'Jpan') {
    const latin = /[A-Za-z0-9]/
    const start = latin.test(text.charAt(0)) ? ' ' : ''
    const end = latin.test(text.charAt(text.length - 1)) ? ' ' : ''
    return `${start}${text}${end}`
  }
  return text
}

/** The name from its parts, the scope's label passed through `scope`; a period of no length is unknown. */
function named(name: QuotaWindowName, scope: (label: string) => string): string {
  const length = name.periodSeconds ? periodWord(name.periodSeconds) : null
  const model = name.scope?.label ?? null
  switch (name.kind) {
    case 'limit':
      if (length !== null && model !== null)
        return t('quota.window.limit.periodScope', { period: length, scope: scope(model) })
      if (length !== null)
        return t('quota.window.limit.period', { period: length })
      if (model !== null)
        return t('quota.window.limit.scope', { scope: scope(model) })
      return t('quota.window.limit.unknown')
    case 'credits':
      return length === null ? t('quota.window.credits.unknown') : t('quota.window.credits.period', { period: length })
    case 'requests':
      return t('quota.window.requests')
    case 'tokens':
      return t('quota.window.tokens')
    case 'cap':
      return t('quota.window.cap')
  }
}

const MINUTE = 60
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
const WEEK = 7 * DAY

/**
 * A period's word: *Daily* for one day, *Weekly* for one week, *Monthly*
 * for 28 to 31 days, otherwise its length in the largest unit that
 * divides it evenly, *5-hour*, *3-day*, *90-minute*.
 */
function periodWord(seconds: number): string {
  if (seconds === DAY)
    return t('quota.window.period.day')
  if (seconds === WEEK)
    return t('quota.window.period.week')
  if (seconds >= 28 * DAY && seconds <= 31 * DAY)
    return t('quota.window.period.month')
  const units = [
    [WEEK, 'quota.window.period.weeks'],
    [DAY, 'quota.window.period.days'],
    [HOUR, 'quota.window.period.hours'],
    [MINUTE, 'quota.window.period.minutes'],
  ] as const
  for (const [unit, key] of units) {
    if (seconds % unit === 0)
      return t(key, { count: seconds / unit })
  }
  return t('quota.window.period.seconds', { count: seconds })
}
