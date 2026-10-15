import { pageLanguage } from '../i18n/text'

/** A count as the page's language writes it, grouped: "1,204". */
export function formatCount(count: number): string {
  return new Intl.NumberFormat(pageLanguage()).format(count)
}

/** `value` with at most `digits` decimals, none when they would be zeros: "12.3", "200". */
export function formatDecimal(value: number, digits: number): string {
  return new Intl.NumberFormat(pageLanguage(), { maximumFractionDigits: digits }).format(value)
}
