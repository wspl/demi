import type { Ref } from 'vue'
import { createSharedComposable, useMediaQuery } from '@vueuse/core'

/**
 * A phone's width: below Tailwind's `md` breakpoint, 768 px. Below it the
 * app frame shows one pane at a time (SidebarLayout): one top bar over the
 * conversation, the sidebar as a drawer, the work panel and the session's
 * windows full screen, and menus as bottom sheets (`Sheet.vue`).
 */
export const PHONE_QUERY = '(max-width: 767px)'

/** Whether the page is a phone's width now, live; one listener for the whole page. */
export const usePhone: () => Readonly<Ref<boolean>> = createSharedComposable(() => useMediaQuery(PHONE_QUERY))

/** Whether the page is a phone's width at this moment, for code that reads it once. */
export function isPhone(): boolean {
  return window.matchMedia(PHONE_QUERY).matches
}
