import { reportCopyFailure } from '../infra/clipboard'
import { showToast } from '../infra/toast'

/** Copies a conversation's id from a menu item, which has no control to show Copied: a toast says so. */
export async function copyConversationId(id: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(id)
    showToast({ title: 'Copied', tone: 'success' })
  } catch (error) {
    reportCopyFailure(error)
  }
}
