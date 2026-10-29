import {
  ClickScrollPlugin,
  OverlayScrollbars,
  type InitializationTargetObject,
  type PartialOptions,
} from 'overlayscrollbars'

/**
 * Every scrollbar in the app is OverlayScrollbars' (`ScrollArea`, and the scrollers
 * other libraries own). It floats over the content and takes no room on any system,
 * shows while the pointer is over its scroller, while it scrolls and while its thumb
 * is dragged, then hides.
 *
 * Its look is the `os-theme-demi` theme in base.css.
 */

// A click on the track scrolls a page towards it, as a native bar does.
OverlayScrollbars.plugin(ClickScrollPlugin)

/** The axes a scroller scrolls along. */
export type ScrollAxis = 'x' | 'y' | 'both'

/** The scrollbars' behaviour for a scroller along `axis`; the other axis clips. */
export function scrollbarsOptions(axis: ScrollAxis): PartialOptions {
  return {
    overflow: {
      x: axis === 'y' ? 'hidden' : 'scroll',
      y: axis === 'x' ? 'hidden' : 'scroll',
    },
    scrollbars: {
      theme: 'os-theme-demi',
      autoHide: 'leave',
      autoHideDelay: 700,
      clickScroll: true,
    },
  }
}

/**
 * Initialisation on an element that scrolls itself and stays the scrolling
 * element: OverlayScrollbars does not wrap its content, so its padding, its
 * layout and whoever reads its scroll position are unchanged. Set it up through
 * {@link inPlace}, which keeps the content where it is.
 *
 * The bars go into `slot`, an element that does not scroll and whose box is the
 * scroller's, or into the scroller itself, where the library keeps them in place
 * as it scrolls.
 */
export function scrollbarsTarget(scroller: HTMLElement, slot: HTMLElement = scroller): InitializationTargetObject {
  return {
    target: scroller,
    elements: { viewport: scroller },
    scrollbars: { slot },
  }
}

/**
 * Runs `initialize`, which sets OverlayScrollbars up on `scroller` at once,
 * leaving the scroller's content in the document.
 *
 * The library appends each child of the scroller to it again, in order. The
 * children end where they were, but each leaves the document for a moment: a
 * frame loads its page again, a video stops, a scroller inside goes back to its
 * start. A bar set up when the user first reaches the scroller, long after its
 * content loaded, would show each of them. While `initialize` runs, appending a
 * child the scroller already has does nothing, which ends with the same order.
 */
export function inPlace<T>(scroller: HTMLElement, initialize: () => T): T {
  const children = new Set<Node>(scroller.childNodes)
  const append = scroller.appendChild
  Object.defineProperty(scroller, 'appendChild', {
    configurable: true,
    value: <N extends Node>(node: N): N => children.has(node) ? node : append.call(scroller, node) as N,
  })
  try {
    return initialize()
  } finally {
    Reflect.deleteProperty(scroller, 'appendChild')
  }
}

/**
 * Overlay scrollbars on a scroller another library owns (CodeMirror's, a
 * rendered document's code block). The caller destroys the instance when the
 * scroller goes.
 */
export function attachScrollbars(scroller: HTMLElement, axis: ScrollAxis, slot?: HTMLElement): OverlayScrollbars {
  return inPlace(scroller, () => OverlayScrollbars(scrollbarsTarget(scroller, slot), scrollbarsOptions(axis)))
}
