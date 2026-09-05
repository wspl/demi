import { t } from '@demicodes/web-ui/infra/i18n'

export function formatThinkingDuration(ms: number): string {
  const s = Math.round(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  const rs = s % 60
  if (m < 60) return rs ? `${m}m${rs}s` : `${m}m`
  const h = Math.floor(m / 60)
  const rm = m % 60
  return rm ? `${h}h${rm}m` : `${h}h`
}

export function thinkingFaceLabel(streaming: boolean, elapsedMs: number | null): string {
  if (elapsedMs === null) return t('agent.block.thinking')
  const prefix = t(streaming ? 'agent.block.thinkingFor' : 'agent.block.thoughtFor')
  return `${prefix} ${formatThinkingDuration(elapsedMs)}`
}
