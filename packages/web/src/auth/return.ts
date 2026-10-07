import type { LocationQuery } from 'vue-router'

/**
 * The page a visitor opened signed out, kept through signing in
 * (`web-application.md` § Authentication), as GitHub and Slack keep theirs:
 * the sign-in address carries it as `next`, and signing in goes on to it.
 */

/** The address that opens when nothing else was asked for. */
const HOME = '/chat'

/** The sign-in page's address for a visitor who opened `page`, and why they are there. */
export function signInAddress(page: string, reason?: 'expired'): string {
  const query = new URLSearchParams()
  if (reason) {
    query.set('reason', reason)
  }
  if (page !== '/' && page !== HOME && !page.startsWith('/login') && !page.startsWith('/setup')) {
    query.set('next', page)
  }
  const search = query.toString()
  return search ? `/login?${search}` : '/login'
}

/**
 * The page to go on to after signing in: the `next` the sign-in address
 * kept, when it is an address of this app; the chat otherwise, so a link
 * cannot send a visitor to another site.
 */
export function returnAddress(query: LocationQuery): string {
  const next = query.next
  if (typeof next !== 'string' || !next.startsWith('/') || next.startsWith('//') || next.startsWith('/\\')) {
    return HOME
  }
  return next
}
