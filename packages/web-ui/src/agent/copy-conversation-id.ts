import { reportError } from '../infra/errors'
import { t } from '../infra/i18n'
import { showToast } from '../infra/toast'

export async function copyConversationId(id: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(id)
    showToast({ title: t('common.copied') })
  } catch (error) {
    reportError('Failed to copy conversation ID', error, { userVisible: true })
  }
}
