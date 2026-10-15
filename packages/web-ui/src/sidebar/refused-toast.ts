import { t } from './locales/text'
import { sliceHead, upperFirst } from '@demicodes/utils'
import { showToast } from '../infra/toast'
import type { HeadlineText, SentenceText } from '../ui/ui-text'

/** A conversation the backend refused to change, by its title, and the backend's reasons, with the request that refused. */
export interface Refusal {
  title: string
  reasons: readonly string[]
  /** The request that refused; none when the page knows of none. */
  requestId?: string | null
}

/** How long a conversation's name may be in a refusal, so the headline stays about one line. */
/** What the backend refused to do to the conversations. */
export type RefusedAction = 'archive' | 'restore' | 'pin' | 'unpin' | 'delete' | 'move'

const NAME_MAX_CHARS = 20

/**
 * A conversation's title as a refusal quotes it: whole when short, else cut
 * at a word with one ellipsis at the end, as the Writing page cuts a name.
 * The backticks a title written as Markdown marks code with, and its closing
 * period, go, so the quote holds only the name.
 */
function quotedName(title: string): string {
  let name = title.replaceAll('`', '').trim()
  if (name.length > NAME_MAX_CHARS) {
    const head = sliceHead(name, NAME_MAX_CHARS - 1)
    const space = head.lastIndexOf(' ')
    name = `${(space > 0 ? head.slice(0, space) : head).replace(/[\s,.;:]+$/, '')}…`
  }
  return t('refused.quote', { name: name.replace(/\.+$/, '') })
}

/** The backend's reason as a sentence: its first letter capitalized, ending with its punctuation. */
function sentence(reason: string): SentenceText {
  const text = upperFirst(reason.trim())
  return /[.?!…]$/.test(text) ? text : `${text}.`
}

/**
 * What a refusal toast says when the backend refused to `action` the
 * conversations `refusals` lists, one entry each (`action` an id:
 * archive, restore, pin, unpin, delete or move): for one, a headline naming it and its
 * reasons as sentences; for several, a headline with their count and each
 * one's name and reasons on a line of its own.
 */
export function refusedText(action: RefusedAction, refusals: readonly Refusal[]): { title: HeadlineText; message: SentenceText } {
  const said = (refusal: Refusal) => refusal.reasons.map(sentence).join(' ')
  if (refusals.length === 1) {
    return { title: t(`refused.${action}.one`, { name: quotedName(refusals[0]!.title) }), message: said(refusals[0]!) }
  }
  return {
    title: t(`refused.${action}.many`, { count: refusals.length }),
    message: refusals.map((refusal) => t('refused.line', { name: quotedName(refusal.title), reasons: said(refusal) })).join('\n'),
  }
}

/**
 * Says in a failure toast that the backend refused to `action` the
 * conversations `refusals` lists, naming the request that refused when one
 * request refused them all.
 */
export function showRefused(action: RefusedAction, refusals: readonly Refusal[]): string {
  const requests = new Set(refusals.map((refusal) => refusal.requestId ?? null))
  const [requestId] = requests
  return showToast({
    ...refusedText(action, refusals),
    tone: 'danger',
    requestId: requests.size === 1 ? requestId : null,
  })
}
