/**
 * The live view of one conversation's browser, for as long as the page shows
 * it (`browser-live-view.md` § Opening a view).
 */
import { onBeforeUnmount, onMounted, shallowRef, toValue, watch, type MaybeRefOrGetter } from 'vue'
import { liveStreamAt } from '../transport/live-stream'
import { viewerClipboard } from './clipboard'
import { viewerPlatform } from './input'
import { LiveSession } from './session'

export interface LiveSessionUse {
  /** A user operation, which the product reports as conversation activity. */
  onOperation?: () => void
}

export function useLiveSession(url: MaybeRefOrGetter<string | null>, options: LiveSessionUse = {}) {
  const session = shallowRef<LiveSession | null>(null)
  const stop = () => {
    session.value?.close()
    session.value = null
  }
  const open = () => {
    stop()
    const next = toValue(url)
    // A hidden page watches nothing: the view opens again when it returns.
    if (!next || document.visibilityState === 'hidden') {
      return
    }
    const live = new LiveSession({
      open: liveStreamAt(next),
      platform: viewerPlatform(navigator),
      onClipboard: (text) => viewerClipboard.receive(text),
      onOperation: options.onOperation,
    })
    session.value = live
    live.start()
  }
  watch(() => toValue(url), open, { immediate: true })
  onMounted(() => document.addEventListener('visibilitychange', open))
  onBeforeUnmount(() => {
    document.removeEventListener('visibilitychange', open)
    stop()
  })
  return session
}
