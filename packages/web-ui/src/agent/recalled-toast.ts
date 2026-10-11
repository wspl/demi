import { showToast } from '../infra/toast'

/**
 * Says that Recall took a waiting message back while the composer held a
 * draft, which it kept as it was, in a toast whose Undo sends the message
 * again the way it was sent (`product.md` § Steer or queue). Answers the
 * toast's id, which a refusal of the take-back dismisses.
 */
export function showRecalled(undo: () => void): string {
  return showToast({
    title: 'Message Recalled',
    tone: 'success',
    action: { label: 'Undo', run: undo },
  })
}
