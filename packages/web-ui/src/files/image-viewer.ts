import { inject, provide, shallowRef, type InjectionKey, type ShallowRef } from 'vue'

/** An image shown large over the page: where it loads from, and what it is called. */
export interface ShownImage {
  src: string
  name: string
}

/** One viewer that shows an image large: the image it shows now, or none. */
export interface ImageViewerState {
  shown: Readonly<ShallowRef<ShownImage | null>>
  show(src: string, name: string): void
  close(): void
}

const imageViewerKey: InjectionKey<ImageViewerState> = Symbol('image-viewer')

/**
 * Gives the components below one viewer that shows an image large over the
 * page (`file-previews.md` § Media a tool returned), which `ImageViewer`
 * draws. A page provides it at its root, so the viewer stays open while the
 * transcript beneath it changes, even when the row that opened it goes.
 */
export function provideImageViewer(): ImageViewerState {
  const shown = shallowRef<ShownImage | null>(null)
  const viewer: ImageViewerState = {
    shown,
    show(src, name) {
      shown.value = { src, name }
    },
    close() {
      shown.value = null
    },
  }
  provide(imageViewerKey, viewer)
  return viewer
}

/** The viewer that shows an image large, which the page provides. */
export function useImageViewer(): ImageViewerState {
  const viewer = inject(imageViewerKey, null)
  if (!viewer) {
    throw new Error('Nothing shows an image large: the page provides a viewer with provideImageViewer')
  }
  return viewer
}
