
export function formatThinkingDuration(ms: number): string {
  const s = Math.round(ms / 1000)
  if (s < 60)
    return `${s}s`
  const m = Math.floor(s / 60)
  const rs = s % 60
  if (m < 60)
    return rs ? `${m}m${rs}s` : `${m}m`
  const h = Math.floor(m / 60)
  const rm = m % 60
  return rm ? `${h}h${rm}m` : `${h}h`
}

export function thinkingFaceLabel(streaming: boolean, elapsedMs: number | null): string {
  if (elapsedMs === null)
    return 'Thinking'
  if (elapsedMs <= 1000)
    return streaming ? 'Thinking' : 'Thought briefly'
  return `${streaming ? 'Thinking for' : 'Thought for'} ${formatThinkingDuration(elapsedMs)}`
}

/**
 * The tail row while the provider is asked: Requesting, or Retrying while the
 * agent asks again on its own, with how long the request has waited.
 */
export function providerWaitLabel(kind: 'requesting' | 'retrying', elapsedMs: number): string {
  const verb = kind === 'retrying' ? 'Retrying' : 'Requesting'
  if (elapsedMs <= 1000)
    return verb
  return `${verb} for ${formatThinkingDuration(elapsedMs)}`
}
