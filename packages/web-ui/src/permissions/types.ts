import { findText, pageLanguage } from '../i18n/text'
import { find, t } from './locales/text'
import { titleCase } from '@demicodes/utils'

/**
 * Conversation requests as the page shows them (`permissions.md` § What the
 * user sees, `credentials.md` § Requests and routes): a request an agent's
 * command raised and the user's answer to it, of one of three kinds, a
 * permission, a credential to save or a stored credential to allow. The host
 * maps its data to these.
 */

/** A permission category: its id, and its action and description while the user's command set declares it. */
export interface PermissionCategoryView {
  id: string
  /** Completes "Allow this conversation to …", such as `manage skills`; none for a category no longer declared. */
  action: string | null
  /** What a grant allows; none for a category no longer declared. */
  description: string | null
}

/** A host a request asks the conversation to reach: a device by the name the user knows it by, or the computer a pairing pairs. */
export type PermissionHostView = { kind: 'device'; deviceId: string; name: string } | { kind: 'pairing' }

/** One undecided request: the command an agent ran without the grants of its categories or the host it brings, at least one of the two. */
export interface PermissionRequestView {
  kind: 'permission'
  id: string
  /** The categories the conversation lacked, in the backend's order. */
  categories: PermissionCategoryView[]
  /** The host the command asks to reach; null for a request of categories alone. */
  host: PermissionHostView | null
  /** The command line as the agent ran it. */
  command: string
  /** The subagent that ran it; null for the root. */
  subagent: { number: number; description: string } | null
}

/** How a credential's field is meant to be used: a secret is typed hidden, a visible one is shown as it is. */
export type CredentialFieldKind = 'secret' | 'visible'

/** A field of a credential: the name an agent reads it by, the label the user sees, and its kind. */
export interface CredentialFieldView {
  name: string
  label: string
  kind: CredentialFieldKind
}

/** How long a credential is kept: a time from its save, or until the user removes it. */
export type CredentialKeep = '1h' | '1d' | '30d' | 'always'

/** A credential an agent asks the user to save: new, or new values of the saved one of that name. */
export interface CredentialRequestView {
  kind: 'credential'
  id: string
  name: string
  /** Why the agent asks; empty when it gave none. */
  reason: string
  fields: CredentialFieldView[]
  /** What *Keep until* shows at first. */
  keep: CredentialKeep
  /** A credential of this name is saved already: the card asks for its fields again. */
  renewal: boolean
}

/** A stored credential this conversation asks to use. */
export interface AllowanceRequestView {
  kind: 'allowance'
  id: string
  name: string
  /** The user removed the credential since it was asked for. */
  removed: boolean
}

/** One undecided request of a conversation, of any kind. */
export type RequestView = PermissionRequestView | CredentialRequestView | AllowanceRequestView

export type PermissionDecision = 'allow' | 'deny'

/** The user's answer to a credential request: each field's value by its name, and how long to keep it. */
export interface CredentialAnswer {
  values: Record<string, string>
  keep: CredentialKeep
}

/** The options of *Keep until*, in the order the card and Settings offer them. */
export const KEEP_OPTIONS: readonly { value: CredentialKeep; label: string }[] = [
  {
    value: '1h',
    get label() {
      return t('keep.1h')
    },
  },
  {
    value: '1d',
    get label() {
      return t('keep.1d')
    },
  },
  {
    value: '30d',
    get label() {
      return t('keep.30d')
    },
  },
  {
    value: 'always',
    get label() {
      return t('keep.always')
    },
  },
]

/**
 * The page's words for the category `id` (`localization.md` § Text from the
 * backend): Demi's own categories' from this catalogue, a plugin's from the
 * plugin's, under its id, which the category's id starts with, as
 * `skills.manage` is the `skills` plugin's. A catalogue key holds no dot, so
 * the id's dots are underscores there. Null for a category no catalogue
 * words, which shows the command set's English.
 */
function categoryText(id: string, field: 'action' | 'title' | 'description'): string | null {
  const key = id.replaceAll('.', '_')
  const plugin = id.split('.')[0]
  return find(`categories.${key}.${field}`) ?? findText(`plugins.${plugin}.permissions.${key}.${field}`)
}

/** What the category lets the conversation do, completing “Allow this conversation to …”: its words, its declared action, or its id when it is no longer declared. */
export function categoryAction(category: Pick<PermissionCategoryView, 'id' | 'action'>): string {
  return categoryText(category.id, 'action') ?? category.action ?? category.id
}

/** What a grant of the category allows; none for a category no longer declared. */
export function categoryDescription(category: PermissionCategoryView): string | null {
  return category.description === null ? null : categoryText(category.id, 'description') ?? category.description
}

/**
 * The actions of the categories `ids`, then the actions `more`, worded
 * already, as one phrase in the page's words; null when a catalogue lacks
 * one of the categories, which then shows its English.
 */
export function wordedActions(ids: readonly string[], more: readonly string[] = []): string | null {
  const actions = ids.map((id) => categoryText(id, 'action'))
  return actions.every((action) => action !== null)
    ? new Intl.ListFormat(pageLanguage(), { type: 'conjunction' }).format([...actions, ...more])
    : null
}

/** What reaching the host lets the conversation do, completing “Allow this conversation to …”: `reach build-box`. */
export function hostAction(host: PermissionHostView): string {
  return host.kind === 'device' ? reachAction(host.name) : t('host.pair')
}

/** What reaching the host named `name` lets the conversation do: `reach build-box`. */
export function reachAction(name: string): string {
  return t('host.reach', { host: name })
}

/** What reaching the host allows, as the card explains it. */
export function hostDescription(host: PermissionHostView): string {
  return host.kind === 'device' ? t('host.description', { host: host.name }) : t('host.pairingDescription')
}

/** The host's title among a request's parts: its name, or *New Computer* for a pairing. */
export function hostTitle(host: PermissionHostView): string {
  return host.kind === 'device' ? host.name : t('host.pairingTitle')
}

/** The card's title: a host alone asks to reach it, anything else to do its parts' actions. */
export function requestTitle(request: Pick<PermissionRequestView, 'categories' | 'host'>): string {
  if (request.categories.length === 0 && request.host) {
    return request.host.kind === 'device'
      ? t('host.allowTitle', { host: request.host.name })
      : t('host.allowPairingTitle')
  }
  return t('request.allowTitle', { action: requestAction(request) })
}

/** What a request to use the saved credential `name` lets the conversation do. */
export function useCredentialAction(name: string): string {
  return t('useCredential', { name })
}

/**
 * What the request lets the conversation do: its categories' actions and
 * its host's joined with "and", as the card's title and the agent's message
 * join them (`permissions.md` § Hosts).
 */
export function requestAction(request: Pick<PermissionRequestView, 'categories' | 'host'>): string {
  const actions = request.categories.map(categoryAction)
  if (request.host) {
    actions.push(hostAction(request.host))
  }
  return new Intl.ListFormat(pageLanguage(), { type: 'conjunction' }).format(actions)
}

/** The category's title, such as “Manage Skills”, as its description names it: its words, else its English action in title case, or its id as it is when it is no longer declared. */
export function categoryTitle(category: PermissionCategoryView): string {
  return categoryText(category.id, 'title') ?? (category.action === null ? category.id : titleCase(category.action))
}

/**
 * The requests left once the user decided `id` (`permissions.md`
 * § Requests): an allow of a permission request grants its categories and
 * its host and decides every permission request they complete, a deny
 * decides that one alone, and a credential or an allowance is decided alone
 * either way. A request that also needs a category granted before, or
 * another host, is left for the backend's answer to decide, since the page
 * knows neither the grants nor the hosts the conversation reaches; so is
 * another pairing, whose code the page does not know.
 */
export function afterDecision(
  requests: readonly RequestView[],
  id: string,
  decision: PermissionDecision,
): RequestView[] {
  const decided = requests.find((request) => request.id === id)
  if (!decided) {
    return [...requests]
  }
  if (decision === 'deny' || decided.kind !== 'permission') {
    return requests.filter((request) => request.id !== id)
  }
  const granted = new Set(decided.categories.map((category) => category.id))
  const reached = (host: PermissionHostView | null) =>
    host === null ||
    (host.kind === 'device' && decided.host?.kind === 'device' && host.deviceId === decided.host.deviceId)
  return requests.filter(
    (request) =>
      request.id !== id &&
      (request.kind !== 'permission' ||
        !request.categories.every((category) => granted.has(category.id)) ||
        !reached(request.host)),
  )
}
