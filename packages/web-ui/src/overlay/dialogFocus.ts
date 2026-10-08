import { onBeforeUnmount, watch, type Ref } from 'vue'
import { onKeyStroke } from '@vueuse/core'
import { useAutofocus } from '../ui/autofocus'
import { useFocusReturn } from './focusReturn'

/** A field the user types in or picks a value from: what a dialog opens on first. */
const FIELD = [
  'input:not([type=button],[type=submit],[type=reset],[type=checkbox],[type=radio],[type=range],[type=color],[type=file],[type=hidden])',
  'textarea',
  'select',
  '[contenteditable=""]',
  '[contenteditable=true]',
  '[role=textbox]',
  '[role=combobox]',
  '[role=searchbox]',
].join(',')

/** A dialog's default button, which Return presses and the dialog opens on when it has no field (`Button` marks it). */
export const DEFAULT_ACTION = '[data-default-action]:not([aria-disabled=true])'

/** Whether the element can take the focus now: shown, enabled, and not in an inert subtree. */
function canFocus(el: Element): el is HTMLElement {
  return el instanceof HTMLElement &&
    !el.matches(':disabled') &&
    el.closest('[inert]') === null &&
    el.checkVisibility()
}

/** The elements Tab reaches inside `root`, in document order. */
function tabStops(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll('*')].filter((el): el is HTMLElement => canFocus(el) && el.tabIndex >= 0)
}

/** The first field, else the default action, else the panel itself. */
function initialFocus(panel: HTMLElement): HTMLElement {
  const field = [...panel.querySelectorAll(FIELD)].find(canFocus)
  if (field) {
    return field
  }
  const action = [...panel.querySelectorAll(DEFAULT_ACTION)].find(canFocus)
  return action ?? panel
}

/** Where Tab or Shift+Tab goes from `active`, among `stops` inside `panel`. */
function nextStop(panel: HTMLElement, stops: HTMLElement[], active: Element | null, forward: boolean): HTMLElement {
  const first = stops[0]!
  const last = stops[stops.length - 1]!
  const index = active instanceof HTMLElement ? stops.indexOf(active) : -1
  if (index >= 0) {
    return stops[(index + (forward ? 1 : stops.length - 1)) % stops.length]!
  }
  // From the panel itself, a button that takes the focus only when given it, or from outside the dialog.
  if (!active || !panel.contains(active)) {
    return forward ? first : last
  }
  if (forward) {
    return stops.find((stop) => active.compareDocumentPosition(stop) & Node.DOCUMENT_POSITION_FOLLOWING) ?? first
  }
  const before = stops.filter((stop) => active.compareDocumentPosition(stop) & Node.DOCUMENT_POSITION_PRECEDING)
  return before[before.length - 1] ?? last
}

/**
 * The focus of a dialog that owns the page, as WAI-ARIA's dialog pattern and
 * macOS sheets have it: when the panel shows, the focus moves into it (its
 * first field, else its default action, else the panel); while it is the top
 * layer, Tab and Shift+Tab go round inside it; when it closes, the focus goes
 * back to what had it when the dialog opened. A close that put the focus
 * somewhere else on purpose keeps it there.
 */
export function useDialogFocus(options: {
  panel: Readonly<Ref<HTMLElement | undefined>>
  isOpen: () => boolean
  /** Whether no other layer (a dialog stacked on it, a menu) stands above it. */
  isTop: () => boolean
}): void {
  const autofocus = useAutofocus()
  const focusReturn = useFocusReturn((active) => options.panel.value?.contains(active) === true)

  // Runs before the panel renders, so the opener is what had the focus, not what the dialog's content took.
  watch(options.isOpen, (open) => {
    if (open) {
      focusReturn.open()
    } else {
      focusReturn.close()
    }
  }, { immediate: true })

  onBeforeUnmount(() => focusReturn.close())

  watch(options.panel, (panel) => {
    // Content that took the focus itself (a field marked `focused`) keeps it.
    if (!panel || panel.contains(document.activeElement)) {
      return
    }
    autofocus(initialFocus(panel))
  }, { flush: 'post' })

  onKeyStroke('Tab', (event) => {
    const panel = options.panel.value
    if (!panel || !options.isOpen() || !options.isTop() || event.defaultPrevented) {
      return
    }
    event.preventDefault()
    const stops = tabStops(panel)
    if (stops.length === 0) {
      if (!panel.contains(document.activeElement)) {
        panel.focus()
      }
      return
    }
    nextStop(panel, stops, document.activeElement, !event.shiftKey).focus()
  })
}
