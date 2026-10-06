import {
  ClickScrollPlugin,
  OverlayScrollbars,
  type InitializationTargetObject,
  type PartialOptions,
} from 'overlayscrollbars'

/**
 * Every scrollbar in the app is OverlayScrollbars' (`ScrollArea`, and the
 * scrollers other libraries own): it floats over the content and takes no
 * room on any system, shows while the pointer is over its scroller, while it
 * scrolls and while its thumb is dragged, then hides. Its look is the
 * `os-theme-demi` theme in base.css.
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
 * element: OverlayScrollbars neither wraps nor moves its content, so its
 * padding, its layout and whoever reads its scroll position are unchanged.
 * The bars go into `slot`, an element that does not scroll and whose box is
 * the scroller's, or into the scroller itself, where the library keeps them
 * in place as it scrolls.
 */
export function scrollbarsTarget(scroller: HTMLElement, slot: HTMLElement = scroller): InitializationTargetObject {
  return {
    target: scroller,
    elements: { viewport: scroller },
    scrollbars: { slot },
  }
}

/**
 * Overlay scrollbars on a scroller another library owns (CodeMirror's, a
 * rendered document's code block). The caller destroys the instance when the
 * scroller goes.
 */
export function attachScrollbars(scroller: HTMLElement, axis: ScrollAxis, slot?: HTMLElement): OverlayScrollbars {
  return OverlayScrollbars(scrollbarsTarget(scroller, slot), scrollbarsOptions(axis))
}
