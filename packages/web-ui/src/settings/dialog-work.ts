import { reportError, type ShownError } from '../infra/errors'
import type { HeadlineText } from '../ui/ui-text'

/** A dialog that edits one thing: whether it is open, and why the last work on it was refused. */
export interface WorkDialog {
  open: boolean
  error: ShownError | null
}

/**
 * Carries out a dialog's work on one thing, such as saving credential
 * *GitHub*: its answer goes only to the dialog it was started from. `shown`
 * finds that dialog, or null once it was closed or another opened in its place.
 * Done, the work closes it; refused, it says why there (`refusal`), or, the
 * dialog gone, a toast titled `title` does, so no refusal goes unsaid.
 */
export async function answerInDialog(
  shown: () => WorkDialog | null,
  title: HeadlineText,
  refusal: (error: unknown) => ShownError,
  work: () => Promise<void>,
): Promise<void> {
  const asking = shown()
  if (asking) {
    asking.error = null
  }
  try {
    await work()
    const done = shown()
    if (done) {
      done.open = false
    }
  } catch (error) {
    const refused = shown()
    if (refused) {
      refused.error = refusal(error)
    } else {
      reportError(title, error, { userVisible: true })
    }
  }
}
