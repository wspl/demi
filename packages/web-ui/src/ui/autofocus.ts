import { hasInjectionContext, inject, type InjectionKey, type Ref } from 'vue'
import { overlayContainerKey } from '../overlay/overlayContainer'
import { focusByPage } from './focus-by-page'
import { isPhone } from './phone'

export const autofocusEnabledKey: InjectionKey<Readonly<Ref<boolean>>> =
  Symbol('autofocus-enabled')

/** A field the user types in or picks a value from: what a dialog opens on first. */
export const FIELD = [
  'input:not([type=button],[type=submit],[type=reset],[type=checkbox],[type=radio],[type=range],[type=color],[type=file],[type=hidden])',
  'textarea',
  'select',
  '[contenteditable=""]',
  '[contenteditable=true]',
  '[role=textbox]',
  '[role=combobox]',
  '[role=searchbox]',
].join(',')

/**
 * What takes the focus by itself: an element, or something that takes it the
 * way its own kind should, such as an editor that puts its cursor at the end,
 * with the element that ends up focused.
 */
export type AutofocusTarget = HTMLElement | { readonly element: HTMLElement; focus(): void }

/**
 * Suppresses automatic focus in a multi-form surface, without blocking user
 * focus.
 *
 * - A surface in a catalog host (`overlayContainerKey`), pinned open beside
 *   others, never takes the focus by itself: the host is not the page's.
 * - A surface the user opened there (`userOpened`, as a dialog is once it is no
 *   longer pinned) takes it as in the product. The page moved the focus, so no
 *   focus ring shows (`focusByPage`).
 * - A surface of the page behind a dialog never takes the focus from it: the
 *   composer that mounts or measures late leaves the focus of a dialog that opened
 *   first (a target that is not in a dialog is behind it).
 *
 * On a phone a field takes no focus as what holds it opens, since the on-screen
 * keyboard would cover what just opened; it waits for a tap. Only a field typing
 * in is the whole of what the user just chose (`asked`: Rename, Search, a filter
 * the user opened) takes it there too.
 *
 * Answers whether the target took the focus, so the caller can put it on its
 * surface instead.
 */
export function useAutofocus(options: { userOpened?: () => boolean, asked?: () => boolean } = {}): (
  target: AutofocusTarget | null | undefined,
  focusOptions?: FocusOptions,
) => boolean {
  const enabled = hasInjectionContext() ? inject(autofocusEnabledKey, null) : null
  const hosted = hasInjectionContext() && inject(overlayContainerKey, null) !== null
  return (target, focusOptions) => {
    if (!target || (hosted && !options.userOpened?.()) || enabled?.value === false) {
      return false
    }
    const element = target instanceof HTMLElement ? target : target.element
    if (behindDialog(element) || (isField(element) && isPhone() && !options.asked?.())) {
      return false
    }
    focusByPage(target, focusOptions)
    return true
  }
}

/** Whether the element takes typing or a picked value, which on a phone brings up the keyboard or a picker. */
function isField(element: HTMLElement): boolean {
  return element.matches(FIELD) || element.isContentEditable
}

/** Whether the focus is in a dialog and `element` is not in one: the page behind it. */
function behindDialog(element: HTMLElement): boolean {
  const inDialog = (node: unknown) => node instanceof Element && node.closest('[role=dialog]') !== null
  return inDialog(document.activeElement) && !inDialog(element)
}
