import { pageLanguage } from '../i18n/text'

/** A count as the page's language writes it, grouped: "1,204". */
export function formatCount(count: number): string {
  return new Intl.NumberFormat(pageLanguage()).format(count)
}

/** `value` with at most `digits` decimals, none when they would be zeros: "12.3", "200". */
export function formatDecimal(value: number, digits: number): string {
  return new Intl.NumberFormat(pageLanguage(), { maximumFractionDigits: digits }).format(value)
}

/**
 * An amount of US dollars as a receipt writes it in the page's language:
 * "$41.20", and an amount under a cent with the digits that say it,
 * "$0.0038", as the backend's own costs read.
 */
export function formatDollars(amount: number): string {
  const small = amount > 0 && amount < 0.01
  return new Intl.NumberFormat(pageLanguage(), {
    style: 'currency',
    currency: 'USD',
    // "$", not "US$", in every language: the card names no other currency.
    currencyDisplay: 'narrowSymbol',
    ...(small ? { maximumSignificantDigits: 2 } : { minimumFractionDigits: 2, maximumFractionDigits: 2 }),
  }).format(amount)
}
