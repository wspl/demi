import { showToast } from '../infra/toast'

/**
 * Says that conversations were archived, in a toast whose Undo brings them
 * back, as Gmail and Linear do (`product.md` § Conversations and projects).
 * Answers the toast's id, which an archive the backend refused dismisses.
 */
export function showArchived(count: number, undo: () => void): string {
  return showToast({
    title: count === 1 ? 'Conversation Archived' : `${count} Conversations Archived`,
    tone: 'success',
    action: { label: 'Undo', run: undo },
  })
}
