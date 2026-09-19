import { reportError } from '../infra/errors'
import { showToast } from '../infra/toast'

export async function copyConversationId(id: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(id)
    showToast({ title: 'Copied', tone: 'success' })
  } catch (error) {
    reportError('Failed to copy conversation ID', error, { userVisible: true })
  }
}
