import { hasInjectionContext, inject, type InjectionKey, type Ref } from 'vue'
import { overlayContainerKey } from '../overlay/overlayContainer'
import { focusByPage } from './focus-by-page'

export const autofocusEnabledKey: InjectionKey<Readonly<Ref<boolean>>> =
  Symbol('autofocus-enabled')

/**
 * Suppresses automatic focus in a multi-form surface, without blocking user
 * focus. The target is an element, or anything that takes the focus the way
 * its own kind should, such as an editor that puts its cursor at the end.
 * A surface in a catalog host (`overlayContainerKey`), pinned open beside
 * others, never takes the focus by itself: the host is not the page's. A
 * surface the user opened there (`userOpened`, as a dialog is once it is
 * no longer pinned) takes it as in the product. The page moved the focus, so
 * no focus ring shows (`focusByPage`). A surface of the page behind a dialog
 * never takes the focus from it: the composer that mounts or measures late
 * leaves the focus of a dialog that opened first (a target that is not in a
 * dialog is behind it).
 */
export function useAutofocus(options: { userOpened?: () => boolean } = {}): (
  target: Pick<HTMLElement, 'focus'> | null | undefined,
  focusOptions?: FocusOptions,
) => void {
  const enabled = hasInjectionContext() ? inject(autofocusEnabledKey, null) : null
  const hosted = hasInjectionContext() && inject(overlayContainerKey, null) !== null
  return (target, focusOptions) => {
    if ((hosted && !options.userOpened?.()) || enabled?.value === false || behindDialog(target)) {
      return
    }
    if (target)
      focusByPage(target, focusOptions)
  }
}

/** Whether the focus is in a dialog and `target` is not in one: the page behind it. */
function behindDialog(target: Pick<HTMLElement, 'focus'> | null | undefined): boolean {
  const inDialog = (element: unknown) => element instanceof Element && element.closest('[role=dialog]') !== null
  return inDialog(document.activeElement) && !inDialog(target)
}
