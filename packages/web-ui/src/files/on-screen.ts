import { inject, provide, watch, type InjectionKey } from 'vue'
import type { Showing } from './file-cache'

/**
 * Whether the views under a component are on screen (`plugin-pages.md`
 * § What the service keeps): a work panel tab the user switched away from
 * keeps its views and what they show, but follows nothing until it is
 * selected again. Outside any such place, views are on screen.
 */
const onScreenKey: InjectionKey<() => boolean> = Symbol('on-screen')

/** The views under the calling component are on screen while `onScreen` and the place around it say so. */
export function provideOnScreen(onScreen: () => boolean): void {
  const outer = useOnScreen()
  provide(onScreenKey, () => outer() && onScreen())
}

export function useOnScreen(): () => boolean {
  return inject(onScreenKey, () => true)
}

/**
 * Keeps `showing()` following its entry only while the calling component's
 * views are on screen: off screen it is away, and back on screen it reads
 * once what reports left unconfirmed meanwhile.
 */
export function followOnScreen(showings: () => readonly (Showing<unknown> | null | undefined)[]): void {
  const onScreen = useOnScreen()
  watch([onScreen, showings], ([shown, held]) => {
    for (const showing of held) {
      if (shown)
        showing?.back()
      else
        showing?.away()
    }
  }, { immediate: true })
}
