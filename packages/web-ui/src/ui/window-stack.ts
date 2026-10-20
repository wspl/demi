import { onBeforeUnmount, shallowRef, watch, type ShallowRef } from 'vue'

/**
 * The session's floating windows, such as the Agents and Running windows
 * and the media viewer, stand in the order they opened, the newest in front: a
 * press in the sidebar or the conversation's column outside every window closes
 * the newest alone, as Escape does, and the press still does what it does
 * there; a second press closes the next. A press in a window closes nothing,
 * nor does one on a place marked with `KEEPS_WINDOWS`: the work panel and its
 * divider (`SidebarLayout`), and the controls that open or change a window,
 * such as the dock's Agents and Running chips and a thumbnail that shows its
 * picture in the viewer in place of the one it shows. The composer and the
 * dock's empty space are outside: a press there closes the newest window and
 * puts the caret in the message.
 */

/** The attribute of a place where a press keeps the session's windows open. */
export const KEEPS_WINDOWS = 'data-keeps-windows'

/** What a press lands on: an element, whose ancestors `closest` reaches. */
export interface PressPlace {
  closest(selectors: string): unknown
}

/** A window of the stack. */
export interface StackedWindow<Place extends PressPlace = Element> {
  /** Whether a press on `place` is in the window. */
  contains(place: Place): boolean
  /** Whether a menu or a dialog is open over the page: a press then closes that first. */
  otherLayerOpen(): boolean
  close(): void
}

/**
 * The window a press on `place` closes: the newest of `windows`, which are
 * in the order they opened; none when the press is in a window, in a place
 * that keeps them, or while a menu or a dialog takes it.
 */
export function windowClosedByPress<Place extends PressPlace>(
  windows: readonly StackedWindow<Place>[],
  place: Place,
): StackedWindow<Place> | null {
  const newest = windows.at(-1)
  if (!newest || newest.otherLayerOpen())
    return null
  if (windows.some((window) => window.contains(place)) || place.closest(`[${KEEPS_WINDOWS}]`) != null)
    return null
  return newest
}

/** The open windows of a page, oldest first. */
export interface WindowStack<Place extends PressPlace = Element> {
  windows: Readonly<ShallowRef<readonly StackedWindow<Place>[]>>
  /** `window` opened, or came to the front by showing something new: it is the newest now. */
  open(window: StackedWindow<Place>): void
  /** `window` closed. */
  remove(window: StackedWindow<Place>): void
  /** A pointer pressed on `place`: closes the window the press closes, if any. */
  press(place: Place): void
}

export function createWindowStack<Place extends PressPlace = Element>(): WindowStack<Place> {
  const windows = shallowRef<readonly StackedWindow<Place>[]>([])
  return {
    windows,
    open(window) {
      windows.value = [...windows.value.filter((other) => other !== window), window]
    },
    remove(window) {
      windows.value = windows.value.filter((other) => other !== window)
    },
    press(place) {
      windowClosedByPress(windows.value, place)?.close()
    },
  }
}

/** The page's windows: the session's and the viewer's are one stack. */
const pageWindows = createWindowStack()

/**
 * Before the press reaches what it lands on, so a window a press opens again,
 * or a link it follows, sees the stack as the press left it.
 */
function onPagePress(event: PointerEvent): void {
  if (event.target instanceof Element)
    pageWindows.press(event.target)
}

// The page listens for presses only while a window is open.
watch(() => pageWindows.windows.value.length > 0, (any) => {
  if (any)
    document.addEventListener('pointerdown', onPagePress, true)
  else
    document.removeEventListener('pointerdown', onPagePress, true)
})

/** What a window primitive tells the stack about itself. */
export interface StackedWindowOptions {
  isOpen: () => boolean
  /** The window's element, which holds every press in it. */
  element: () => HTMLElement | null | undefined
  otherLayerOpen: () => boolean
  close: () => void
}

/**
 * Puts a window primitive in the page's stack while it is open, until it
 * unmounts. `isNewest` says whether it stands in front, where Escape is its;
 * `raise` brings it to the front when it shows something new.
 */
export function useStackedWindow(options: StackedWindowOptions): { isNewest: () => boolean; raise: () => void } {
  const entry: StackedWindow = {
    contains: (place) => options.element()?.contains(place) === true,
    otherLayerOpen: options.otherLayerOpen,
    close: options.close,
  }
  watch(options.isOpen, (open) => {
    if (open)
      pageWindows.open(entry)
    else
      pageWindows.remove(entry)
  }, { immediate: true })
  onBeforeUnmount(() => pageWindows.remove(entry))
  return {
    isNewest: () => pageWindows.windows.value.at(-1) === entry,
    raise: () => {
      if (options.isOpen())
        pageWindows.open(entry)
    },
  }
}
