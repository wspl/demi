import { computed, inject, provide, shallowRef, type ComputedRef, type InjectionKey, type ShallowRef } from 'vue'
import {
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
  /** The transcript row it stands in, which tells apart the same picture standing in two rows (`transcriptRowOf`). */
  row?: string
}

/** The row of the transcript the element stands in, as a list marks its rows; none outside a transcript. */
export function transcriptRowOf(element: Element | null): string | undefined {
  return element?.closest<HTMLElement>('[data-block-id]')?.dataset['blockId']
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

/** What the window sizes itself against: the page, and the display's device pixels per page pixel. */
export interface ViewerPlace {
  page: PageSize
  pixelRatio: number
}

/**
 * The media of the transcript a medium was opened from, in the order they
 * stand, read at each use so that it follows the transcript. The same
 * medium may stand in it more than once.
 */
export type MediaSequence = () => readonly ShownMedium[]

/** Where the medium shown stands among its transcript's: its place from 1, and how many there are. */
export interface ViewerPosition {
  at: number
  of: number
}

/**
 * One viewer that shows a medium large, for the whole page
 * (`file-previews.md` § The media viewer): the medium it shows now, or none,
 * and where its window stands. The window takes its size from the medium
 * (`windowSizeFor`) and opens centred on the page, whether it is the first
 * medium or one shown while the window is open; a size the user dragged
 * lasts while that medium shows. A medium opened from a transcript is
 * stepped to the one before it or after it (`step`).
 */
export interface MediaViewerState {
  shown: Readonly<ShallowRef<ShownMedium | null>>
  /** Where the window stands; it outlives a close, so the window can leave. */
  geometry: Readonly<ShallowRef<ViewerGeometry | null>>
  /** The shown medium's place among its transcript's media; null for one opened on its own. */
  position: Readonly<ComputedRef<ViewerPosition | null>>
  /**
   * Shows `medium`, in place of the one shown, in the same window.
   * `among` gives the media of the transcript it stands in, which `step` goes through.
   */
  show(medium: ShownMedium, among?: MediaSequence): void
  /** Shows the medium before (-1) or after (1) the one shown in its transcript; nothing at the first and the last. */
  step(direction: -1 | 1): void
  close(): void
  /** The user moved or resized the window. */
  place(rect: WindowRect): void
  /** The medium shown has loaded at `size`: a window that opened without it takes it now. */
  measured(size: PixelSize): void
}

/** The state of the viewer. `where` tells the page and the display's pixel ratio. */
export function createMediaViewer(where: () => ViewerPlace): MediaViewerState {
  const shown = shallowRef<ShownMedium | null>(null)
  const geometry = shallowRef<ViewerGeometry | null>(null)
  const sequence = shallowRef<MediaSequence | null>(null)
  /** Where the medium shown stood last, which tells the same medium apart where the transcript shows it twice. */
  const index = shallowRef(-1)

  /** The window for a medium of `size`, centred on the page. */
  function sized(size: PixelSize | null): WindowRect {
    const { page, pixelRatio } = where()
    return windowRectAround(windowSizeFor(size, pixelRatio, page), { x: page.width / 2, y: page.height / 2 }, page)
  }

  /**
   * Where `medium` stands in `media`: at `last`, where it stood, if it still
   * does, else its first place in its row, else its first place; -1 if it is
   * not there.
   */
  function locate(media: readonly ShownMedium[], medium: ShownMedium, last: number): number {
    if (media[last]?.src === medium.src)
      return last
    const inRow = medium.row === undefined ? -1 : media.findIndex((each) => each.src === medium.src && each.row === medium.row)
    return inRow >= 0 ? inRow : media.findIndex((each) => each.src === medium.src)
  }

  const position = computed<ViewerPosition | null>(() => {
    const medium = shown.value
    const media = sequence.value?.()
    if (!medium || !media)
      return null
    const at = locate(media, medium, index.value)
    return at < 0 ? null : { at: at + 1, of: media.length }
  })

  /** Shows `medium` in the window, standing at `at` of the transcript's media, if it is known. */
  function open(medium: ShownMedium, among: MediaSequence | null, at: number): void {
    // The same medium again leaves the window as the user left it.
    if (shown.value?.src !== medium.src || !geometry.value) {
      const size = medium.size ?? null
      geometry.value = { rect: sized(size), awaitsSize: size === null }
    }
    shown.value = medium
    sequence.value = among
    index.value = at
  }

  return {
    shown,
    geometry,
    position,
    show(medium, among) {
      open(medium, among ?? null, among ? locate(among(), medium, -1) : -1)
    },
    step(direction) {
      const place = position.value
      const among = sequence.value
      if (!place || !among)
        return
      const to = place.at - 1 + direction
      const next = among()[to]
      if (next)
        open(next, among, to)
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
      geometry.value = { rect: sized(size), awaitsSize: false }
    },
  }
}

/** The page's viewport, and the display's pixel ratio. */
function pagePlace(): ViewerPlace {
  return {
    page: { width: window.innerWidth, height: window.innerHeight },
    pixelRatio: window.devicePixelRatio,
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
