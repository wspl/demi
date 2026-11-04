/**
 * What a view's page tells the window that frames it, to any origin: the
 * link is the key, and the message holds nothing of it. A page opened on its
 * own has no parent to tell.
 */

/** Why a view ended for good. */
export type ViewEnd = 'expired' | 'removed' | 'not_found' | 'tab_closed'

export type ViewMessage =
  | { type: 'demi.view'; state: 'live' }
  | { type: 'demi.view'; state: 'ended'; reason: ViewEnd }

export function tellParent(message: ViewMessage): void {
  if (window.parent !== window) {
    window.parent.postMessage(message, '*')
  }
}

/** How a view's stream ended, when it ended for good; null for an end it opens again after. */
export function finalEnd(reason: string): ViewEnd | null {
  switch (reason) {
    case 'view_expired':
      return 'expired'
    case 'view_removed':
      return 'removed'
    case 'view_not_found':
      return 'not_found'
    default:
      return null
  }
}
