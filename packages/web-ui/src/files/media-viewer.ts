import { inject, provide, shallowRef, type InjectionKey, type ShallowRef } from 'vue'

/**
 * An image or a video shown large over the page: what it is, where it loads
 * from, and what it is called.
 */
export interface ShownMedium {
  kind: 'image' | 'video'
  src: string
  name: string
}

/** One viewer that shows a medium large: the medium it shows now, or none. */
export interface MediaViewerState {
  shown: Readonly<ShallowRef<ShownMedium | null>>
  show(medium: ShownMedium): void
  close(): void
}

const mediaViewerKey: InjectionKey<MediaViewerState> = Symbol('media-viewer')

/**
 * Gives the components below one viewer that shows an image or a video large
 * over the page (`file-previews.md` § Media a tool returned), which
 * `MediaViewer` draws. A page provides it at its root, so the viewer stays
 * open while the transcript beneath it changes, even when the row that
 * opened it goes.
 */
export function provideMediaViewer(): MediaViewerState {
  const shown = shallowRef<ShownMedium | null>(null)
  const viewer: MediaViewerState = {
    shown,
    show(medium) {
      shown.value = medium
    },
    close() {
      shown.value = null
    },
  }
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
