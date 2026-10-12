import { formatSpan } from '@demicodes/utils'

export function thinkingFaceLabel(streaming: boolean, elapsedMs: number | null): string {
  if (elapsedMs === null)
    return 'Thinking'
  if (elapsedMs <= 1000)
    return streaming ? 'Thinking' : 'Thought briefly'
  return `${streaming ? 'Thinking for' : 'Thought for'} ${formatSpan(elapsedMs)}`
}

/** The tail row while the provider is asked, with how long the request has waited. */
export function providerWaitLabel(elapsedMs: number): string {
  if (elapsedMs <= 1000)
    return 'Requesting'
  return `Requesting for ${formatSpan(elapsedMs)}`
}
