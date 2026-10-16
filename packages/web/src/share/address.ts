/**
 * The share a shared page's address names: `/s/<token>`, the link the share
 * dialog copies (`web-api-conversations.md` § Sharing); null for any other address.
 */
export function shareToken(pathname: string): string | null {
  const token = /^\/s\/([^/]+)\/?$/.exec(pathname)?.[1]
  return token === undefined ? null : decodeURIComponent(token)
}
