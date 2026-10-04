/**
 * Conversation permissions as the page shows them (`permissions.md` § What
 * the user sees): a request an agent's command raised, the user's answer to
 * it, and a category the user allowed. The host maps its data to these.
 */

/** A permission category: its id, and its action and description while the user's command set declares it. */
export interface PermissionCategoryView {
  id: string
  /** Completes "Allow this conversation to …", such as `manage skills`; none for a category no longer declared. */
  action: string | null
  /** What a grant allows; none for a category no longer declared. */
  description: string | null
}

/** One undecided request: the command an agent ran without the grant of its category. */
export interface PermissionRequestView {
  id: string
  category: PermissionCategoryView
  /** The command line as the agent ran it. */
  command: string
  /** The subagent that ran it; null for the root. */
  subagent: { number: number; description: string } | null
}

/** A category the user allowed for the conversation. */
export interface PermissionGrantView {
  category: PermissionCategoryView
  /** When it was granted, as an ISO time. */
  grantedAt: string
}

export type PermissionDecision = 'allow' | 'deny'

/** What the category lets the conversation do: its action, or its id when it is no longer declared. */
export function categoryAction(category: PermissionCategoryView): string {
  return category.action ?? category.id
}

/** The category's title: its action with its first letter capitalized, such as "Manage skills"; its id as it is when it is no longer declared. */
export function categoryTitle(category: PermissionCategoryView): string {
  if (category.action === null) {
    return category.id
  }
  return category.action.charAt(0).toUpperCase() + category.action.slice(1)
}

/**
 * The requests left once the user decided `id` (`permissions.md`
 * § Requests): an allow decides every request of its category, a deny that
 * one alone.
 */
export function afterDecision(
  requests: readonly PermissionRequestView[],
  id: string,
  decision: PermissionDecision,
): PermissionRequestView[] {
  const decided = requests.find((request) => request.id === id)
  if (!decided) {
    return [...requests]
  }
  return requests.filter((request) =>
    decision === 'allow'
      ? request.category.id !== decided.category.id
      : request.id !== id,
  )
}
