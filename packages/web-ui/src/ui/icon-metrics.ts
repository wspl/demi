/**
 * Live-measured icon sizes. Do not use Lucide defaults (16 / 24).
 *
 * Toolbar chips: 28/14 default, 32/16 large, 24/14 compact — icon is half the hit.
 * Checkbox box is 14. The tick is a 2px stroke in that box, not a Lucide toolbar icon.
 * Nav / activity rows: 14 in 28. Brand marks in those rows: 16.
 * Stroke stays `--icon-stroke-width: 1.75` (≈1px optical at 14 in a 24 viewBox).
 */
export const ICON_PX = {
  in12: 10,
  in20: 12,
  in24: 14,
  in28: 14,
  markIn28: 16,
  in32: 16,
} as const
