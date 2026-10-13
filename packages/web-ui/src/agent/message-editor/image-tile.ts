import { shallowRef, watch, type Ref, type ShallowRef } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { fitMessageMedium, messagePicture, type MessagePicture } from '../../markdown/media-run'

/**
 * Makes the image a sent message names a tile
 * (`file-previews.md` § Files named in messages), sized as the agent's
 * message sizes its images (`markdown/media-run.ts`): when it mounts, when
 * its bytes arrive, which give its size (the caller calls `fit` on `load`,
 * for the first address and for each new version), and when the width of the
 * block it stands in changes. The block is the image's parent. Does nothing where `enabled` is false, as in the
 * composer, which keeps its image in the line. Returns the picture the hover preview shows, with
 * the size the image has by then; null before the image is there.
 */
export function useImageTile(image: Ref<HTMLImageElement | undefined>, enabled: boolean): {
  picture: ShallowRef<MessagePicture | null>
  /** Sizes the image now: its bytes arrived. */
  fit(): void
} {
  const picture = shallowRef<MessagePicture | null>(null)
  function fit(): void {
    const element = image.value
    if (!enabled || !element)
      return
    fitMessageMedium(element)
    picture.value = messagePicture(element)
  }
  // The image is there; the browser reports its bytes (`fit` on `load`).
  watch(image, fit, { flush: 'post' })
  useResizeObserver(() => image.value?.parentElement, fit)
  return { picture, fit }
}
