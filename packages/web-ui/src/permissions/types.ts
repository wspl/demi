import { findText, pageLanguage } from '../i18n/text'
import en from './locales/en.json'
import { find, t } from './locales/text'
import { categoryKey, deviceTitleIn, requestTitleIn, type RequestTitleWords } from './title-words'

/**
 * Conversation requests as the page shows them: a request an agent's command raised
 * and the user's answer to it. A request is one of two kinds: a permission or a
 * credential to save.
 *
 * The host maps its data to these.
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
  kind: 'permission'
  id: string
  /** The category the conversation lacked; none for a device's request. */
  categories: PermissionCategoryView[]
  /** The command line as the agent ran it. */
  command: string
  /** The subagent that ran it; null for the root. */
  subagent: { number: number; description: string } | null
  /** The device the command brings in, which an allow attaches; null for a request of a category. */
  device: { id: string; name: string } | null
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
  /** How long the credential is kept once saved, as the agent decided; the card says it. */
  keep: CredentialKeep
  /** A credential of this name is saved already: the card asks for its fields again. */
  renewal: boolean
}

/** One undecided request of a conversation, of any kind. */
export type RequestView = PermissionRequestView | CredentialRequestView

export type PermissionDecision = 'allow' | 'deny'

/** The user's answer to a credential request: each field's value by its name, kept as the request's `keep` says. */
export interface CredentialAnswer {
  values: Record<string, string>
}

/**
 * The page's words for the category `id`: Demi's own categories' from this
 * catalogue, a plugin's from the plugin's, under its id, which the category's id
 * starts with, as `skills.manage` is the `skills` plugin's.
 *
 * A catalogue key holds no dot, so the id's dots are underscores there. Null for a
 * category no catalogue words, which shows the command set's English.
 */
function categoryText(id: string, field: 'action' | 'description'): string | null {
  const key = categoryKey(id)
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

/** What a device's decision lets the conversation do: `use “build-box”`. */
export function useDeviceAction(name: string): string {
  return t('device.use', { name })
}

/** What a decision recorded before a permission covered all devices let the conversation do by a pairing. */
export function pairAction(): string {
  return t('device.pair')
}

/**
 * The words of the cards' titles, with the actions of `categories`; of
 * every category this package's catalogue words when none are given.
 */
export function requestTitleWords(categories?: readonly Pick<PermissionCategoryView, 'id' | 'action'>[]): RequestTitleWords {
  const actions: Record<string, string> = {}
  if (categories) {
    for (const category of categories) {
      actions[categoryKey(category.id)] = categoryAction(category)
    }
  } else {
    for (const key of Object.keys(en.categories)) {
      const action = find(`categories.${key}.action`)
      if (action !== null) {
        actions[key] = action
      }
    }
  }
  return {
    allow: t('request.allowTitle', { action: '{action}' }),
    actions,
    useDevice: t('device.title', { name: '{name}' }),
  }
}

/** The card's title: the use of its device, or its category's action, asked. */
export function requestTitle(request: Pick<PermissionRequestView, 'categories' | 'device'>): string {
  const words = requestTitleWords(request.categories)
  return request.device ? deviceTitleIn(words, request.device.name) : requestTitleIn(words, request)
}

/** What a decision recorded before a permission covered all credentials let the conversation do with the credential `name`. */
export function useCredentialAction(name: string): string {
  return t('useCredential', { name })
}

/**
 * The requests left once the user decided `id`:
 *
 * - An allow of a permission request grants its category and decides every
 *   permission request of the same category; of a device's request, every
 *   request of that device.
 * - A deny decides that one alone.
 * - A credential request is decided alone either way.
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
  const same = (request: PermissionRequestView) =>
    decided.device
      ? request.device?.id === decided.device.id
      : request.device === null && request.categories[0]?.id === decided.categories[0]?.id
  return requests.filter((request) => request.id !== id && (request.kind !== 'permission' || !same(request)))
}
