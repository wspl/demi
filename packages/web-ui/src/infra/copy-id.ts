import { reportCopyFailure } from './clipboard'
import { showToast } from './toast'
import { t as shared } from '../locales/text'

/**
 * Copies what names an object (a conversation's id, an agent's number, a
 * file's path, a commit's hash) from a menu item, which has no control to
 * show Copied: a toast says so. A name still to be read, as a commit's full
 * hash from its abbreviation, is written as the clipboard item's promise,
 * inside the choice that asks for it, as Safari requires; one that cannot be
 * read is told as a copy that failed.
 */
export async function copyId(id: string | Promise<string>): Promise<void> {
  try {
    if (typeof id === 'string') {
      await navigator.clipboard.writeText(id)
    } else {
      const blob = id.then((text) => new Blob([text], { type: 'text/plain' }))
      // A name that could not be read rejects first, and says why rather than the clipboard.
      await Promise.all([id, navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob })])])
    }
    showToast({ title: shared('common.copied'), tone: 'success' })
  } catch (error) {
    reportCopyFailure(error)
  }
}
