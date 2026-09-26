import { showToast } from '@demicodes/web-ui/infra/toast'

/**
 * What the product would do with a Host or another page, said in a neutral
 * toast: a specimen's answer to a control whose effect lies outside it, since
 * every control in the gallery responds when used (`AGENTS.md`).
 */
export function productWould(title: string): void {
  showToast({ title, tone: 'neutral' })
}
