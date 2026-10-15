import { errorMessage } from './errors'
import { t } from './locales/text'
import { showToast } from './toast'

/**
 * Tells the user a copy did not reach the clipboard, the one way every copy
 * in the app does: a toast titled Could Not Copy that gives the reason.
 */
export function reportCopyFailure(error: unknown): void {
  showToast({
    title: t('copyFailed'),
    message: () => errorMessage(error),
    tone: 'danger',
  })
}
