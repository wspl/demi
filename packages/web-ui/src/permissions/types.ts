import { upperFirst } from '@demicodes/utils'

/**
 * Conversation permissions as the page shows them (`permissions.md` § What
 * the user sees): a request an agent's command raised and the user's answer
 * to it. The host maps its data to these.
 */

/** A permission category: its id, and its action and description while the user's command set declares it. */
export interface PermissionCategoryView {
  id: string
  /** Completes "Allow this conversation to …", such as `manage skills`; none for a category no longer declared. */
  action: string | null
  /** What a grant allows; none for a category no longer declared. */
  description: string | null
}

/** One undecided request: the command an agent ran without the grants of its categories, one or more. */
export interface PermissionRequestView {
  id: string
  /** The categories the conversation lacked, in the backend's order. */
  categories: PermissionCategoryView[]
  /** The command line as the agent ran it. */
  command: string
  /** The subagent that ran it; null for the root. */
  subagent: { number: number; description: string } | null
}

export type PermissionDecision = 'allow' | 'deny'

/** What the category lets the conversation do: its action, or its id when it is no longer declared. */
export function categoryAction(category: PermissionCategoryView): string {
  return category.action ?? category.id
}

/**
 * What the request lets the conversation do: its categories' actions joined
 * with "and", as the card's title and the agent's message join them
 * (`permissions.md` § Several categories).
 */
export function requestAction(request: Pick<PermissionRequestView, 'categories'>): string {
  return request.categories.map(categoryAction).join(' and ')
}

/** The category's title: its action with its first letter capitalized, such as "Manage skills"; its id as it is when it is no longer declared. */
export function categoryTitle(category: PermissionCategoryView): string {
  if (category.action === null) {
    return category.id
  }
  return upperFirst(category.action)
}

/**
 * The requests left once the user decided `id` (`permissions.md`
 * § Requests): an allow grants the request's categories and decides every
 * request they complete, a deny decides that one alone. A request that also
 * needs a category granted before is left for the backend's answer to
 * decide, since the page does not know the grants.
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
  if (decision === 'deny') {
    return requests.filter((request) => request.id !== id)
  }
  const granted = new Set(decided.categories.map((category) => category.id))
  return requests.filter((request) => !request.categories.every((category) => granted.has(category.id)))
}
