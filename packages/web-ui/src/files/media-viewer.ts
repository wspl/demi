import { inject, provide, shallowRef, type InjectionKey, type ShallowRef } from 'vue'
import { paneBox } from '../ui/region'
import {
  windowCentre,
  windowRectAround,
  windowSizeFor,
  type PageSize,
  type WindowRect,
} from '../ui/floating-window'
import type { PixelSize } from './thumbnail'

/**
 * An image or a video shown large over the page: what it is, where it loads
 * from, what it is called, and what the page knows of it before it loads.
 */
export interface ShownMedium {
  kind: 'image' | 'video'
  src: string
  name: string
  /** Its pixel size, when its reference or its thumbnail knows it. */
  size?: PixelSize | null
  /** What opened it, a thumbnail or a link: the window first opens centred over its pane. */
  from?: Element | null
}

/**
 * Where the viewer's window stands. `awaitsSize` holds while the window
 * shows a medium whose size was not known when it opened: a measured size
 * resizes the window then, and the user's move or resize ends the wait.
 */
export interface ViewerGeometry {
  rect: WindowRect
  awaitsSize: boolean
}

/**
 * What the window sizes itself against: the page, the display's device
 * pixels per page pixel, and the pane centred under the window when it opens.
 */
export interface ViewerPlace {
  page: PageSize
  pixelRatio: number
  over: WindowRect
}

/**
 * One viewer that shows a medium large, for the whole page
 * (`file-previews.md` § The media viewer): the medium it shows now, or none,
 * and where its window stands. The window takes its size from the medium
 * (`windowSizeFor`): the first one opens centred over the conversation,
 * another shown while it is open resizes it about its centre, and a size the
 * user dragged lasts while that medium shows.
 */
export interface MediaViewerState {
  shown: Readonly<ShallowRef<ShownMedium | null>>
  /** Where the window stands; it outlives a close, so the window can leave. */
  geometry: Readonly<ShallowRef<ViewerGeometry | null>>
  /** Shows `medium`, in place of the one shown, in the same window. */
  show(medium: ShownMedium): void
  close(): void
  /** The user moved or resized the window. */
  place(rect: WindowRect): void
  /** The medium shown has loaded at `size`: a window that opened without it takes it now. */
  measured(size: PixelSize): void
}

/**
 * The viewer's state. `where` tells the page, the display's pixel ratio and
 * the pane a medium opened from an element is centred over.
 */
export function createMediaViewer(where: (from: Element | null) => ViewerPlace): MediaViewerState {
  const shown = shallowRef<ShownMedium | null>(null)
  const geometry = shallowRef<ViewerGeometry | null>(null)

  /** The window for a medium of `size`: about the centre of the open `current`, else centred over its pane. */
  function sized(size: PixelSize | null, from: Element | null, current: WindowRect | null): WindowRect {
    const { page, pixelRatio, over } = where(from)
    return windowRectAround(windowSizeFor(size, pixelRatio, page), windowCentre(current ?? over), page)
  }

  return {
    shown,
    geometry,
    show(medium) {
      // The same medium again leaves the window as the user left it.
      if (shown.value?.src !== medium.src || !geometry.value) {
        const size = medium.size ?? null
        const open = shown.value !== null ? geometry.value?.rect ?? null : null
        geometry.value = { rect: sized(size, medium.from ?? null, open), awaitsSize: size === null }
      }
      shown.value = medium
    },
    close() {
      shown.value = null
    },
    place(rect) {
      geometry.value = { rect, awaitsSize: false }
    },
    measured(size) {
      const current = geometry.value
      if (!current?.awaitsSize || size.width <= 0 || size.height <= 0)
        return
      geometry.value = { rect: sized(size, shown.value?.from ?? null, current.rect), awaitsSize: false }
    },
  }
}

/** The page's viewport, and the pane around `from`, the conversation for a thumbnail in a window over it. */
function pagePlace(from: Element | null): ViewerPlace {
  return {
    page: { width: window.innerWidth, height: window.innerHeight },
    pixelRatio: window.devicePixelRatio,
    over: paneBox(from),
  }
}

const mediaViewerKey: InjectionKey<MediaViewerState> = Symbol('media-viewer')

/**
 * Gives the components below the page's one viewer, which `MediaViewer`
 * draws. A page provides it at its root, so the viewer stays open while the
 * transcript beneath it changes, even when the row that opened it goes.
 */
export function provideMediaViewer(): MediaViewerState {
  const viewer = createMediaViewer(pagePlace)
  provide(mediaViewerKey, viewer)
  return viewer
}

/** The viewer that shows a medium large, which the page provides. */
export function useMediaViewer(): MediaViewerState {
  const viewer = inject(mediaViewerKey, null)
  if (!viewer) {
    throw new Error('Nothing shows a medium large: the page provides a viewer with provideMediaViewer')
  }
  return viewer
}
