import { showToast } from '../infra/toast'

/**
 * Says that conversations were archived, in a toast whose Undo brings them
 * back, as Gmail and Linear do (`product.md` § Conversations and projects).
 */
export function showArchived(count: number, undo: () => void): void {
  showToast({
    title: count === 1 ? 'Conversation Archived' : `${count} Conversations Archived`,
    tone: 'success',
    action: { label: 'Undo', run: undo },
  })
}
