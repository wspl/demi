import { showToast } from '../infra/toast'

/**
 * Says that Undo Send took a waiting message back while the composer held a
 * draft, which it kept as it was, in a toast whose Undo sends the message
 * again the way it was sent (`product.md` § Steer or queue). Answers the
 * toast's id, which a refusal of the take-back dismisses.
 */
export function showUnsent(undo: () => void): string {
  return showToast({
    title: 'Message Unsent',
    tone: 'success',
    action: { label: 'Undo', run: undo },
  })
}
