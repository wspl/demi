import { showToast, type ToastTone } from './toast'
import type { HeadlineText, SentenceText, TitleText } from '../ui/ui-text'

/**
 * What a page of another build than the backend serves says
 * (`web-application.md` § A page of another build): the toast that asks the
 * user to reload it, whose one action is Reload.
 */
export const OUTDATED_PROMPT: {
  readonly title: HeadlineText
  readonly message: SentenceText
  readonly tone: ToastTone
  readonly action: TitleText
} = {
  title: 'Demi Was Updated',
  message: 'This page is from an earlier version. Reload it to go on.',
  tone: 'neutral',
  action: 'Reload',
}

/** Shows the prompt until the user reloads or closes it; `reload` loads the page again. */
export function showOutdated(reload: () => void): string {
  return showToast({
    title: OUTDATED_PROMPT.title,
    message: OUTDATED_PROMPT.message,
    tone: OUTDATED_PROMPT.tone,
    durationMs: 0,
    action: { label: OUTDATED_PROMPT.action, run: reload },
  })
}
