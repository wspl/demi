import type { Ref } from 'vue'
import { useMediaQuery } from '@vueuse/core'

/**
 * Whether the user's device is a touch phone or tablet without a pointer or
 * keyboard of its own: no pointer can hover, and the one it has is a finger.
 * Such a device has no keyboard shortcuts to set, and a field focused on its
 * own would open the on-screen keyboard over the page.
 */
export function useTouchOnly(): Readonly<Ref<boolean>> {
  return useMediaQuery('(hover: none) and (pointer: coarse)')
}
