import { pageLanguage } from '../i18n/text'

/**
 * A share as a meter's label reads it: whole percent, rounded down, so a
 * quota shows full ("100%") only once it is, and a vendor's float
 * (7.000000000000001 of 100) reads "7%".
 */
const WHOLE_PERCENT = new Intl.NumberFormat('en-US', {
  style: 'percent',
  maximumFractionDigits: 0,
  roundingMode: 'floor',
})

/** `value` of `max` as whole percent: "7%"; 0% when `max` is not positive. */
export function formatPercent(value: number, max: number): string {
  return WHOLE_PERCENT.format(max > 0 ? Math.min(1, Math.max(0, value / max)) : 0)
}

/** A cache hit rate, a share of 0 to 1, to two decimals: "91.25%". */
export function formatHitRate(rate: number): string {
  return new Intl.NumberFormat(pageLanguage(), {
    style: 'percent',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(rate)
}
