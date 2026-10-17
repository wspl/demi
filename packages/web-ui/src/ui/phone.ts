import type { Ref } from 'vue'
import { createSharedComposable, useMediaQuery } from '@vueuse/core'

/**
 * The phone layout: a page narrower than Tailwind's `md` breakpoint, 768 px,
 * and the one owner of that switch (CSS reads it as the `phone:` variant of
 * `base.css`, the same query). Below it the app frame shows one pane at a
 * time (SidebarLayout): one top bar over the conversation, the sidebar as a
 * drawer, the work panel and the session's windows full screen, menus as
 * bottom sheets (`Sheet.vue`); rows are a finger's height, and what a hover
 * reveals shows at rest. It is the only switch for touch behavior: a touch
 * screen in the desktop layout gets the desktop's.
 */
export const PHONE_QUERY = '(max-width: 767px)'

/**
 * A phone's screen edges are the app's: a touch that starts this close to
 * the left or right edge never takes the browser back or forward
 * (`phone-page.ts`), and one at the left edge that moves right opens the
 * sidebar's drawer (SidebarLayout).
 */
export const SCREEN_EDGE_PX = 16

/** Whether the page is a phone's width now, live; one listener for the whole page. */
export const usePhone: () => Readonly<Ref<boolean>> = createSharedComposable(() => useMediaQuery(PHONE_QUERY))

/** Whether the page is a phone's width at this moment, for code that reads it once. */
export function isPhone(): boolean {
  return window.matchMedia(PHONE_QUERY).matches
}
