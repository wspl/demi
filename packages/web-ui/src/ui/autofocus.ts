import { hasInjectionContext, inject, type InjectionKey, type Ref } from 'vue'
import { overlayContainerKey } from '../overlay/overlayContainer'

export const autofocusEnabledKey: InjectionKey<Readonly<Ref<boolean>>> =
  Symbol('autofocus-enabled')

/**
 * Suppresses automatic focus in a multi-form surface, without blocking user
 * focus. The target is an element, or anything that takes the focus the way
 * its own kind should, such as an editor that puts its cursor at the end.
 * A surface in a catalog host (`overlayContainerKey`), pinned open beside
 * others, never takes the focus by itself: the host is not the page's. A
 * surface the user opened there (`userOpened`, as a dialog is once it is
 * no longer pinned) takes it as in the product.
 */
export function useAutofocus(options: { userOpened?: () => boolean } = {}): (
  target: Pick<HTMLElement, 'focus'> | null | undefined,
) => void {
  const enabled = hasInjectionContext() ? inject(autofocusEnabledKey, null) : null
  const hosted = hasInjectionContext() && inject(overlayContainerKey, null) !== null
  return (target) => {
    if ((hosted && !options.userOpened?.()) || enabled?.value === false) {
      return
    }
    target?.focus()
  }
}
