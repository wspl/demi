import { computed, inject, provide, ref, shallowRef, watch, type ComputedRef, type InjectionKey, type Ref, type ShallowRef } from 'vue'
import { hostImage } from './host-image'
import { useShowing } from './showing'
import {
  windowRectAround,
  windowSizeFor,
  type PageSize,
  type WindowRect,
} from '../ui/floating-window'
import type { PixelSize } from './thumbnail'
import type { FileContents } from './types'

/** A Host file a medium shows, which the viewer shows at the version the page keeps of it and follows while it shows it. */
export interface MediumFile {
  path: string
  contents: FileContents
}

/**
 * An image or a video shown large over the page: what it is, where it loads
 * from, its address or, for a Host image, its file, what it is called, and
 * what the page knows of it before it loads.
 */
export type ShownMedium = {
  kind: 'image' | 'video'
  name: string
  /** Its pixel size, when its reference or its thumbnail knows it. */
  size?: PixelSize | null
  /** Its size in bytes, when its reference knows it; a Host file's comes from the file. */
  bytes?: number | null
  /** The transcript row it stands in, which tells apart the same picture standing in two rows (`transcriptRowOf`). */
  row?: string
} & (
  | { src: string; file?: never }
  | { file: MediumFile; src?: never }
)

/**
 * Where a medium stands in a conversation's series, as a tab that shows it
 * keeps it across a reload: its key (`mediumKey`) and the row it stands in.
 */
export interface MediumPlace {
  key: string
  row?: string
}

/** What tells a medium from another: its address, or a Host image's file, whatever version shows. */
export function mediumKey(medium: ShownMedium): string {
  return medium.file ? `file:${medium.file.path}` : medium.src
}

/**
 * Where the viewer loads `medium` from while it is `open`: its address, or for a
 * Host image the version of its file the page keeps.
 *
 * A Host image shows from the kept files while the viewer is open, so it is the
 * picture the message shows and the file's next version replaces it in place.
 * Until a medium first shows, a description the Host reported changed is not
 * shown: the viewer waits for its new reading, so an outdated picture never
 * shows first.
 * `gone` says a Host image's file is not there. Closed, the last stays for the
 * window's leave. `bytes` is the size of what shows: a Host image's file's, or
 * the one its reference knows.
 */
export function useMediumSource(
  medium: () => ShownMedium | null,
  open: () => boolean,
): { src: Readonly<Ref<string>>; gone: Readonly<Ref<boolean>>; bytes: Readonly<Ref<number | null>> } {
  const file = useShowing(
    () => (open() ? medium()?.file?.contents : null),
    () => medium()?.file?.path,
    (contents, path) => contents.showDescription(path),
  )
  const src = ref('')
  const gone = ref(false)
  const bytes = ref<number | null>(null)
  watch(medium, (shown) => {
    src.value = shown?.src ?? ''
    gone.value = false
    bytes.value = shown?.bytes ?? null
  }, { immediate: true })
  watch(() => {
    const shownFile = medium()?.file
    return shownFile && open()
      ? { image: hostImage(file.entry.value, shownFile.contents, shownFile.path), outdated: file.entry.value?.outdated ?? false }
      : null
  }, (shown) => {
    // Closed, or a medium that is no file: what shows stays.
    if (shown === null)
      return
    const { image, outdated } = shown
    // Nothing shown yet of this medium: a description the Host reported
    // changed waits for its new reading.
    if (image.state === 'shown' && !(src.value === '' && outdated)) {
      src.value = image.src
      bytes.value = file.entry.value?.value?.size ?? bytes.value
    }
    gone.value = image.state === 'none'
  }, { immediate: true })
  return { src, gone, bytes }
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
 * One viewer that shows a medium large, for the whole page: the medium it shows
 * now, or none, and where its window stands.
 *
 * The window takes its size from the medium (`windowSizeFor`) and opens centred
 * on the page, whether it is the first medium or one shown while the window is
 * open. A size the user dragged lasts while that medium shows. A medium opened
 * from a transcript is stepped to the one before it or after it (`step`).
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
  /**
   * Whether the medium shown is in the work panel's Media tab, which the
   * page's `reveal` opened, rather than in the viewer's window.
   */
  inPanel: Readonly<ShallowRef<boolean>>
  /**
   * Offers `series`, the media of a conversation's own transcript, in which
   * a restored Media tab finds the medium it showed (`restore`); the newest
   * offer first. Returns the withdrawal, for the transcript's end.
   */
  offer(series: MediaSequence): () => void
  /**
   * Shows the medium at `place` of the offered series in the tab that asks,
   * as it was shown before a reload; false while no series offered has it.
   */
  restore(place: MediumPlace): boolean
  /** The user moved or resized the window. */
  place(rect: WindowRect): void
  /** The medium shown has loaded at `size`: a window that opened without it takes it now. */
  measured(size: PixelSize): void
}

/**
 * Where a viewer shows its media: in a window it draws over the page, or,
 * with `reveal`, in the work panel's Media tab, which `reveal` opens and
 * selects each time a medium is shown.
 */
export interface MediaViewerOptions {
  /**
   * Opens and selects the Media tab; false when no tab opens it, as with
   * the file-browser plugin off, and the window shows the medium then.
   */
  reveal?(): boolean
}

/** The state of the viewer. `where` tells the page and the display's pixel ratio. */
export function createMediaViewer(where: () => ViewerPlace, { reveal }: MediaViewerOptions = {}): MediaViewerState {
  const shown = shallowRef<ShownMedium | null>(null)
  const geometry = shallowRef<ViewerGeometry | null>(null)
  const sequence = shallowRef<MediaSequence | null>(null)
  /** Where the medium shown stood last, which tells the same medium apart where the transcript shows it twice. */
  const index = shallowRef(-1)
  const inPanel = shallowRef(false)
  /** The series the conversations' own transcripts offer, the newest last. */
  const offered = shallowRef<readonly MediaSequence[]>([])

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
    return locatePlace(media, { key: mediumKey(medium), ...(medium.row === undefined ? {} : { row: medium.row }) }, last)
  }

  /** `locate` by the medium's place. */
  function locatePlace(media: readonly ShownMedium[], { key, row }: MediumPlace, last: number): number {
    const at = media[last]
    if (at && mediumKey(at) === key)
      return last
    const inRow = row === undefined ? -1 : media.findIndex((each) => mediumKey(each) === key && each.row === row)
    return inRow >= 0 ? inRow : media.findIndex((each) => mediumKey(each) === key)
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
    if (!shown.value || mediumKey(shown.value) !== mediumKey(medium) || !geometry.value) {
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
    inPanel,
    show(medium, among) {
      open(medium, among ?? null, among ? locate(among(), medium, -1) : -1)
      inPanel.value = reveal?.() ?? false
    },
    offer(series) {
      offered.value = [...offered.value, series]
      return () => {
        offered.value = offered.value.filter((each) => each !== series)
      }
    },
    restore(place) {
      for (const series of offered.value.toReversed()) {
        const media = series()
        const at = locatePlace(media, place, -1)
        const medium = media[at]
        if (medium) {
          open(medium, series, at)
          inPanel.value = true
          return true
        }
      }
      return false
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
 * draws, or the work panel's Media tab (`MediaView`) when `options` reveal
 * it. A page provides it at its root, or a conversation's page where it has
 * a work panel, so the viewer stays open while the transcript beneath it
 * changes, even when the row that opened it goes.
 */
export function provideMediaViewer(options: MediaViewerOptions = {}): MediaViewerState {
  const viewer = createMediaViewer(pagePlace, options)
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
