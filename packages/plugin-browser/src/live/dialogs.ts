/**
 * A page's JavaScript dialog as a browser shows it (`live-view.md` § A
 * browser tab in the panel): titled with the page's host, answered with
 * Enter and Escape.
 */
import type { LiveDialog } from '../generated/plugin'

/** The dialog's title: who asks, or, for leaving the page, the browser's own question. */
export function dialogTitle(type: LiveDialog['type'], host: string): string {
  if (type === 'beforeunload') {
    return 'Leave the page?'
  }
  return host ? `${host} says` : 'This page says'
}

/**
 * The answer a key gives the dialog: Enter its default button, Escape its
 * Cancel, or OK for an alert, which has nothing else; null for any other key.
 */
export function keyAnswer(key: string, type: LiveDialog['type']): boolean | null {
  if (key === 'Enter') {
    return true
  }
  if (key === 'Escape') {
    return type === 'alert'
  }
  return null
}
