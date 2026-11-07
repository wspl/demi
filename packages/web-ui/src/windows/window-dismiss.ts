import { inject, provide, type InjectionKey } from 'vue'

/**
 * How a view in a phone's window leaves it for the conversation, as a picture
 * flicked down or a tap beside it does: the window minimizes. Null beside the
 * conversation on a computer, where a window stays until the user puts it
 * away.
 */
const windowDismissKey: InjectionKey<() => (() => void) | null> = Symbol('windowDismiss')

export function provideWindowDismiss(dismiss: () => (() => void) | null): void {
  provide(windowDismissKey, dismiss)
}

/** What puts away the window a view stands in, or null where a view does not put its window away. */
export function useWindowDismiss(): () => (() => void) | null {
  return inject(windowDismissKey, () => null)
}
