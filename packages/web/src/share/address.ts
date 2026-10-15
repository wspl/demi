/**
 * The share a shared page's address names: `/s/<token>`, the link the share
 * dialog copies (`web-api.md` § Sharing); null for any other address.
 */
export function shareToken(pathname: string): string | null {
  const token = /^\/s\/([^/]+)\/?$/.exec(pathname)?.[1]
  return token === undefined ? null : decodeURIComponent(token)
}

/**
 * The subagent whose transcript a shared page's address shows, by the
 * number its `agent:` links name: `#agent-4`, as a click on such a link
 * puts it, so the browser's Back returns to the conversation; null for the
 * conversation's own.
 */
export function shownAgent(hash: string): number | null {
  const number = /^#agent-([1-9]\d*)$/.exec(hash)?.[1]
  return number === undefined ? null : Number(number)
}

/** The fragment that shows agent `number`, or the conversation's own for null. */
export function agentFragment(number: number | null): string {
  return number === null ? '' : `#agent-${number}`
}
