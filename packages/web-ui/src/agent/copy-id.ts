import { reportCopyFailure } from '../infra/clipboard'
import { showToast } from '../infra/toast'
import { t as shared } from '../locales/text'

/** Copies an id (a conversation's, or an agent's number) from a menu item, which has no control to show Copied: a toast says so. */
export async function copyId(id: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(id)
    showToast({ title: shared('common.copied'), tone: 'success' })
  } catch (error) {
    reportCopyFailure(error)
  }
}
