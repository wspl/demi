import { showToast } from '../infra/toast'
import { t } from './locales/text'
import { t as shared } from '../locales/text'

/**
 * Says that Recall took a waiting message back while the composer held a
 * draft, which it kept as it was, in a toast whose Undo sends the message
 * again the way it was sent (`turns.md` § Steer or queue). Answers the
 * toast's id, which a refusal of the take-back dismisses.
 */
export function showRecalled(undo: () => void): string {
  return showToast({
    title: t('transcript.user.recalled'),
    tone: 'success',
    action: { label: shared('common.undo'), run: undo },
  })
}
