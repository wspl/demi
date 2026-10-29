import { formatSpan } from '../ui/dates'
import { t } from './locales/text'

/**
 * What a thinking row says: while it streams, for how long so far; once it
 * ended, for how long it took, or only that it took place when its block
 * does not record its end.
 */
export function thinkingFaceLabel(streaming: boolean, elapsedMs: number | null): string {
  if (elapsedMs === null)
    return streaming ? t('transcript.thinking.now') : t('transcript.thinking.done')
  if (elapsedMs <= 1000)
    return streaming ? t('transcript.thinking.now') : t('transcript.thinking.briefly')
  const span = formatSpan(elapsedMs)
  return streaming ? t('transcript.thinking.nowFor', { span }) : t('transcript.thinking.doneFor', { span })
}

/**
 * The tail row while the provider is asked, with how long the request has
 * waited.
 */
export function providerWaitLabel(elapsedMs: number): string {
  if (elapsedMs <= 1000)
    return t('transcript.thinking.requesting')
  return t('transcript.thinking.requestingFor', { span: formatSpan(elapsedMs) })
}
