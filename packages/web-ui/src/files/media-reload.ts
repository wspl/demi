import { onBeforeUnmount, readonly, ref, watch } from 'vue'
import { t } from './locales/text'
import { browserOnline, pageVisibility } from '../transport/liveness'

/**
 * The page's images and videos that did not load, loaded again.
 *
 * A browser tries a medium's address once: one whose bytes did not come, as
 * on a phone whose page returned before its network did, or while Demi
 * restarted, stays broken until the page loads again. So each medium that
 * failed loads again whenever its bytes may come now: when the page reaches
 * the backend again (`reloadFailedMedia`, which the app calls), when the page
 * is shown again, and when the browser comes back online. One that fails
 * again waits for the next of these.
 *
 * - A component that swaps a failed medium for its own placeholder, as a
 *   tool's picture does, shows the medium again at each `mediaReloads`.
 * - Rendered HTML, as a message's, loads each failed medium again in place
 *   (`useFailedMedia`), and marks an attachment's tile meanwhile
 *   (`MEDIA_FAILED_ATTRIBUTE`).
 */
const reloads = ref(0)

/** How many times the page has loaded its failed media again. */
export const mediaReloads = readonly(reloads)

/** Loads every medium that did not load again, as the page reaches the backend again. */
export function reloadFailedMedia(): void {
  reloads.value += 1
}

watch(pageVisibility, (visibility) => {
  if (visibility === 'visible')
    reloadFailedMedia()
})
watch(browserOnline, (online) => {
  if (online)
    reloadFailedMedia()
})

/**
 * The attribute of an attachment's tile in a message whose medium did not
 * load: the tile keeps its box and shows the placeholder a tool's picture
 * shows (`styles/base.css`), never the browser's broken picture.
 */
export const MEDIA_FAILED_ATTRIBUTE = 'data-media-failed'

/** The attachment's tile a medium stands in, which its failure marks; none for another image. */
function tileOf(medium: HTMLImageElement | HTMLVideoElement): Element | null {
  return medium.closest('a[data-attachment-image], a[data-attachment-video]')
}

/** Marks `medium`'s tile as failed, its name kept and why it shows nothing said, or unmarks it. */
function markTile(medium: HTMLImageElement | HTMLVideoElement, failed: boolean): void {
  const tile = tileOf(medium)
  if (!tile)
    return
  if (failed) {
    tile.setAttribute(MEDIA_FAILED_ATTRIBUTE, '')
    tile.setAttribute('aria-description', medium instanceof HTMLVideoElement ? t('view.videoFailed') : t('view.imageFailed'))
  } else {
    tile.removeAttribute(MEDIA_FAILED_ATTRIBUTE)
    tile.removeAttribute('aria-description')
  }
}

function asMedium(target: EventTarget | null): HTMLImageElement | HTMLVideoElement | null {
  return target instanceof HTMLImageElement || target instanceof HTMLVideoElement ? target : null
}

/**
 * The failed media of rendered HTML: `onError` and `onLoaded` take the capture
 * of its root's `error` and its media's `load` and `loadedmetadata`. Each
 * failed medium is marked until it loads, and loads again at each
 * `mediaReloads` while it stands in the page.
 */
export function useFailedMedia(): { onError(event: Event): void; onLoaded(event: Event): void } {
  const failed = new Set<HTMLImageElement | HTMLVideoElement>()
  const stop = watch(reloads, () => {
    for (const medium of failed) {
      if (!medium.isConnected) {
        failed.delete(medium)
        continue
      }
      const src = medium.getAttribute('src')
      if (src === null)
        continue
      // Setting the address again loads it again; a player loads anew.
      if (medium instanceof HTMLVideoElement)
        medium.load()
      else
        medium.setAttribute('src', src)
    }
  })
  onBeforeUnmount(stop)
  return {
    onError(event) {
      const medium = asMedium(event.target)
      if (!medium)
        return
      // A render that replaced the message's nodes left the media it took out.
      for (const held of failed) {
        if (!held.isConnected)
          failed.delete(held)
      }
      failed.add(medium)
      markTile(medium, true)
    },
    onLoaded(event) {
      const medium = asMedium(event.target)
      if (!medium || !failed.delete(medium))
        return
      markTile(medium, false)
    },
  }
}
