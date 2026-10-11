import { inject, onBeforeUnmount, onMounted, type InjectionKey } from 'vue'

/**
 * The app frame's sidebar as a page inside it reaches it on a phone
 * (SidebarLayout): there the frame shows one top bar, and a page with a bar
 * of its own, such as the conversation's, puts the sidebar's button in it;
 * the frame's own bar, the button and the frame's label, shows only while
 * no page does.
 */
export interface SidebarDrawer {
  /** Opens the sidebar's drawer. */
  open(): void
  /** A page shows its own top bar: the frame hides its own until the returned release. */
  claimBar(): () => void
}

export const sidebarDrawerKey: InjectionKey<SidebarDrawer> = Symbol('sidebarDrawer')

/**
 * For a page that shows its own top bar on a phone: opens the frame's
 * drawer, and keeps the frame's own bar hidden while the page is there.
 * Null outside an app frame, where the page has no sidebar to open.
 */
export function useSidebarDrawer(): { open: () => void } | null {
  const drawer = inject(sidebarDrawerKey, null)
  if (!drawer)
    return null
  let release: (() => void) | null = null
  onMounted(() => {
    release = drawer.claimBar()
  })
  onBeforeUnmount(() => {
    release?.()
    release = null
  })
  return { open: () => drawer.open() }
}
