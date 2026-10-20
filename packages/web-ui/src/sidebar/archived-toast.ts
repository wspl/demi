import { t } from './locales/text'
import { showToast, updateToast } from '../infra/toast'
import type { HeadlineText } from '../ui/ui-text'

function archivedTitle(count: number): HeadlineText {
  return t('archivedToast.title', { count }, count)
}

/**
 * Says that conversations were archived, in a toast whose Undo brings them
 * back, as Gmail and Linear do. Answers the toast's id, which an archive the
 * backend refused for some corrects (`updateArchived`) and one refused for all
 * dismisses.
 */
export function showArchived(count: number, undo: () => void): string {
  return showToast({
    title: archivedTitle(count),
    tone: 'success',
    action: { label: t('archivedToast.undo'), run: undo },
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
    action: { label: t('archivedToast.undo'), run: undo },
  })
}
