import { computed, type ComputedRef } from 'vue'
import { usePhone } from './phone'

/**
 * Live-measured icon sizes. Do not use Lucide defaults (16 / 24).
 *
 * - Toolbar chips: 28/14 default, 32/16 large, 24/14 compact — icon is half the
 *   hit.
 * - Checkbox box is 14. The tick is a 2px stroke in that box, not a Lucide toolbar
 *   icon.
 * - Nav / activity rows: 14 in 28. Brand marks in those rows: 16.
 * - Stroke stays `--icon-stroke-width: 1.75` (≈1px optical at 14 in a 24 viewBox).
 *
 * The keys name the desktop's control each icon sits in; a phone draws the same
 * controls larger (`PHONE_ICON_PX`).
 */
export const ICON_PX = {
  in12: 10,
  in20: 12,
  in24: 14,
  in28: 14,
  markIn28: 16,
  in32: 16,
  /** A row's icon beside its text: a work row, a sidebar row, a menu's. */
  row: 14,
  /** A chip's or a pill's icon beside its text. */
  chip: 14,
} as const

export type IconMetrics = { readonly [Key in keyof typeof ICON_PX]: number }

/**
 * The same icons on a phone (`base.css`, the phone's metrics):
 *
 * - The 28 px control is a 40 px toolbar button there with a 20 px icon.
 * - The 24 px one is an answer's 36 px action with 18.
 * - The 20 px one is 28 with 16.
 * - A 44 px row's icon is 18 beside its 16–17 px text, a 32 px chip's 16.
 * - The 32 px control keeps its size, and its icon.
 */
export const PHONE_ICON_PX: IconMetrics = {
  in12: 12,
  in20: 16,
  in24: 18,
  in28: 20,
  markIn28: 20,
  in32: 16,
  row: 18,
  chip: 16,
}

/** The icon sizes of the layout the page is in now, live: the phone's below 768 px. */
export function useIconPx(): ComputedRef<IconMetrics> {
  const phone = usePhone()
  return computed(() => (phone.value ? PHONE_ICON_PX : ICON_PX))
}
