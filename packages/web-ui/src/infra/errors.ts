import { showToast } from './toast'
import type { HeadlineText } from '../ui/ui-text'

export interface ReportErrorOptions {
  userVisible?: boolean
  expected?: boolean
  detail?: string
}

export function reportError(
  title: HeadlineText,
  error: unknown,
  options: ReportErrorOptions = {}
): void {
  const message = error instanceof Error ? error.message : String(error)
  if (!options.expected)
    console.error(`[demi] ${title}: ${message}`, options.detail ?? '')
  if (!options.userVisible)
    return
  showToast({ title, message, tone: 'danger' })
}
