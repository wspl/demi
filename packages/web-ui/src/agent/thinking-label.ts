import { t } from '@demicodes/web-ui/infra/i18n'

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
    return t('agent.block.thinking')
  if (elapsedMs <= 1000)
    return t(streaming ? 'agent.block.thinking' : 'agent.block.thoughtBriefly')
  const prefix = t(streaming
    ? 'agent.block.thinkingFor'
    : 'agent.block.thoughtFor')
  return `${prefix} ${formatThinkingDuration(elapsedMs)}`
}

/**
 * The tail row while the provider is asked: Requesting, or Retrying while the
 * agent asks again on its own, with how long the request has waited.
 */
export function providerWaitLabel(kind: 'requesting' | 'retrying', elapsedMs: number): string {
  const [short, long] = kind === 'retrying'
    ? ['agent.block.retrying', 'agent.block.retryingFor']
    : ['agent.block.requesting', 'agent.block.requestingFor']
  if (elapsedMs <= 1000)
    return t(short)
  return `${t(long)} ${formatThinkingDuration(elapsedMs)}`
}
