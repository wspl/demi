import { toValue, type MaybeRefOrGetter } from 'vue'
import { useEventListener } from '@vueuse/core'

export function eventHitsInside(
  event: Event,
  target: EventTarget | null | undefined,
): boolean {
  if (!target) {
    return false
  }
  return event.target === target || event.composedPath().includes(target)
}

function isMatchable(node: EventTarget): node is Element {
  return typeof (node as Element).matches === 'function'
}

export function eventHitsSelector(event: Event, selector: string): boolean {
  return event
    .composedPath()
    .some((node) => node != null && isMatchable(node) && node.matches(selector))
}

/** True when the event is outside `target` and every ignore entry. */
export function shouldDismissOutside(
  event: Event,
  target: EventTarget | null | undefined,
  ignore: ReadonlyArray<string | EventTarget | null | undefined> = [],
): boolean {
  if (!target) {
    return false
  }
  if (eventHitsInside(event, target)) {
    return false
  }
  for (const item of ignore) {
    if (typeof item === 'string') {
      if (eventHitsSelector(event, item)) {
        return false
      }
      continue
    }
    if (eventHitsInside(event, item)) {
      return false
    }
  }
  return true
}

/** Mouse primary-button down: focus and activation start here, before `click`. */
export function isPrimaryMousePointerDown(event: PointerEvent): boolean {
  return (
    event.isPrimary !== false &&
    event.button === 0 &&
    event.pointerType !== 'touch' &&
    event.pointerType !== 'pen'
  )
}

export function consumeEvent(event: Event): void {
  if (event.cancelable) {
    event.preventDefault()
  }
  event.stopPropagation()
}

/**
 * Close a floating window from a pointer outside it, and do not deliver that
 * gesture to what is underneath. Floating menus (`[data-overlay-panel]`) and
 * dock chips that toggle the window (`[data-session-overlay-toggle]`) stay
 * ignorable. Touch and pen still scroll: those dismiss on `click` only.
 */
export function onDismissOutside(
  target: MaybeRefOrGetter<EventTarget | null | undefined>,
  dismiss: () => void,
  options?: {
    ignore?: MaybeRefOrGetter<ReadonlyArray<string | EventTarget | null | undefined>>
  },
): void {
  let swallowClick = false
  let swallowTimer: ReturnType<typeof setTimeout> | undefined

  function armSwallow(): void {
    swallowClick = true
    if (swallowTimer !== undefined) {
      clearTimeout(swallowTimer)
    }
    swallowTimer = setTimeout(() => {
      swallowClick = false
      swallowTimer = undefined
    }, 0)
  }

  function isIgnored(event: Event): boolean {
    return !shouldDismissOutside(
      event,
      toValue(target),
      toValue(options?.ignore ?? []),
    )
  }

  useEventListener(
    window,
    'pointerdown',
    (event: PointerEvent) => {
      if (isIgnored(event) || !isPrimaryMousePointerDown(event)) {
        return
      }
      consumeEvent(event)
      armSwallow()
      dismiss()
    },
    { capture: true },
  )

  useEventListener(
    window,
    'click',
    (event: MouseEvent) => {
      if (swallowClick) {
        consumeEvent(event)
        swallowClick = false
        return
      }
      if (isIgnored(event)) {
        return
      }
      consumeEvent(event)
      dismiss()
    },
    { capture: true },
  )
}
