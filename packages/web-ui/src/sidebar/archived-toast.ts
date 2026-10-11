import { showToast, updateToast } from '../infra/toast'
import type { HeadlineText } from '../ui/ui-text'

function archivedTitle(count: number): HeadlineText {
  return count === 1 ? 'Conversation Archived' : `${count} Conversations Archived`
}

/**
 * Says that conversations were archived, in a toast whose Undo brings them
 * back, as Gmail and Linear do (`product.md` § Conversations and projects).
 * Answers the toast's id, which an archive the backend refused for some
 * corrects (`updateArchived`) and one refused for all dismisses.
 */
export function showArchived(count: number, undo: () => void): string {
  return showToast({
    title: archivedTitle(count),
    tone: 'success',
    action: { label: 'Undo', run: undo },
  })
}

/**
 * Says in the archive's toast `id`, in place, that `count` were archived
 * once the backend refused the others, and its Undo then brings back only
 * those, with `undo`.
 */
export function updateArchived(id: string, count: number, undo: () => void): void {
  updateToast(id, {
    title: archivedTitle(count),
    action: { label: 'Undo', run: undo },
  })
}
