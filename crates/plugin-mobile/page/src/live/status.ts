import { computed } from 'vue'
import { downloadProgress, formatByteRatio, type HostDownload } from '@demicodes/plugin-sdk'
import type { DeviceWait, LiveConnection, PicturelessReason } from './session'
import { t } from '../locales/text'

/** What a view's status reads of the stream its pictures come on: the device's own, or one display's (`./display-pictures.ts`). */
export interface ShownPictures {
  readonly state: {
    connection: LiveConnection
    pictureless: PicturelessReason | null
    frameFailed: boolean
    look: { picture: unknown } | null
  }
  readonly waiting: DeviceWait
  readonly reconnecting: boolean
  retryFrame(): void
}

/**
 * What a device's view says of its stream, in every view of a device: what
 * it waits for before the first picture, how far the Host's download of the
 * plugin's program came meanwhile, and a stream that stalls or opens again
 * while the last picture shows.
 */
export function deviceStatus(props: { session: ShownPictures; downloads: readonly HostDownload[] }) {
  const look = computed(() => props.session.state.look)
  /** How far the Host's download came, while the stream waits for it; null when there is none. */
  const progress = computed(() => downloadProgress(props.downloads))
  /** How far the download came, in words, or a blank line that keeps its room. */
  const progressText = computed(() => progress.value ? formatByteRatio(progress.value.received, progress.value.size) : ' ')
  /** What the view waits for, in words, while it shows no picture. */
  const waitingLabel = computed(() => {
    switch (props.session.waiting) {
      case 'connecting':
        return progress.value ? t('region.downloading') : t('region.connecting')
      case 'reconnecting':
        return progress.value ? t('region.downloading') : t('region.reconnecting')
      case 'frame':
        return t('region.frame')
      case 'screen':
        return t('region.screen')
      // A failure is no wait: the view says it and offers Retry.
      default:
        return null
    }
  })
  const stalled = computed(() => props.session.state.connection === 'stalled')
  /** The stream ended and opens again, while the view still shows the last picture. */
  const reconnecting = computed(() => props.session.reconnecting && !!look.value?.picture)
  return { progress, progressText, waitingLabel, stalled, reconnecting }
}
