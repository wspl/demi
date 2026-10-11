import { inject, provide, shallowRef, type InjectionKey, type ShallowRef } from 'vue'
import { paneBox } from '../ui/region'
import { initialWindowRect, type PageSize, type WindowRect } from '../ui/floating-window'
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
 * Where the viewer's window stands. `fitted` holds while the window still
 * waits to take the size of the first medium it shows, whose size was not
 * known when it opened; a measured size or the user's move ends it.
 */
export interface ViewerGeometry {
  rect: WindowRect
  fitted: boolean
}

/** The page the window opens on, and the pane centred under it the first time. */
export interface ViewerPlace {
  page: PageSize
  over: WindowRect
}

/**
 * One viewer that shows a medium large, for the whole page
 * (`file-previews.md` § The media viewer): the medium it shows now, or none,
 * and where its window stands, which outlives a close so the next medium
 * opens where the user left the last one.
 */
export interface MediaViewerState {
  shown: Readonly<ShallowRef<ShownMedium | null>>
  geometry: Readonly<ShallowRef<ViewerGeometry | null>>
  /** Shows `medium`, in place of the one shown, in the window where it stands. */
  show(medium: ShownMedium): void
  close(): void
  /** The user moved or resized the window. */
  place(rect: WindowRect): void
  /** The medium shown has loaded at `size`: a first window opened without it takes it now. */
  measured(size: PixelSize): void
}

/**
 * The viewer's state. `where` tells where the window would first open for
 * a medium opened from an element; it is asked only for the first medium,
 * and again when that one's size arrives.
 */
export function createMediaViewer(where: (from: Element | null) => ViewerPlace): MediaViewerState {
  const shown = shallowRef<ShownMedium | null>(null)
  const geometry = shallowRef<ViewerGeometry | null>(null)

  function fit(size: PixelSize | null, from: Element | null): WindowRect {
    const { page, over } = where(from)
    return initialWindowRect(size, page, over)
  }

  return {
    shown,
    geometry,
    show(medium) {
      if (!geometry.value) {
        const size = medium.size ?? null
        geometry.value = { rect: fit(size, medium.from ?? null), fitted: size === null }
      }
      shown.value = medium
    },
    close() {
      shown.value = null
    },
    place(rect) {
      geometry.value = { rect, fitted: false }
    },
    measured(size) {
      if (!geometry.value?.fitted)
        return
      geometry.value = { rect: fit(size, shown.value?.from ?? null), fitted: false }
    },
  }
}

/** The page's viewport, and the pane around `from`, the conversation for a thumbnail in a window over it. */
function pagePlace(from: Element | null): ViewerPlace {
  return { page: { width: window.innerWidth, height: window.innerHeight }, over: paneBox(from) }
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
