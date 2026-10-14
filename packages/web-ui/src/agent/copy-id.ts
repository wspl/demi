import { reportCopyFailure } from '../infra/clipboard'
import { showToast } from '../infra/toast'

/** Copies an id (a conversation's, or an agent's number) from a menu item, which has no control to show Copied: a toast says so. */
export async function copyId(id: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(id)
    showToast({ title: 'Copied', tone: 'success' })
  } catch (error) {
    reportCopyFailure(error)
  }
}
