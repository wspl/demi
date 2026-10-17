import { z } from 'zod'
import { fillText, truncate } from '@demicodes/utils'
import { markdownPlainText } from '@demicodes/web-ui/markdown/plain-text'
import { requestTitleIn } from '@demicodes/web-ui/permissions/title-words'
import { pushMessageSchema, type PushMessage } from '../api/generated/web-api'

/**
 * A push notification as the page's service worker shows it (`turns.md`
 * § Notifications): the backend sends values, and the worker words them in
 * the page's language from the words the page last left it, since no page
 * may run when a push comes. Shared by the page, which leaves the words,
 * and the worker, which reads them; it imports nothing that needs a page.
 */

/** Where the page leaves the words for its worker. */
export const WORDS_CACHE = 'demi-notifications'
export const WORDS_KEY = '/words'

/** What a page tells the worker of a click: open the conversation. */
export const OPEN_CONVERSATION = 'demi-open-conversation'

/** How much of the answer a notification says. */
const ANSWER_START_CHARS = 120

/**
 * The words a notification is said in: a conversation without a title,
 * a failed turn's line, a credential to save, with `{name}` for its name,
 * and the requests' cards' titles (`RequestTitleWords`).
 */
export const notificationWordsSchema = z.object({
  untitled: z.string(),
  turnStopped: z.string(),
  saveCredential: z.string(),
  allow: z.string(),
  reach: z.string(),
  pairing: z.string(),
  useCredential: z.string(),
  actions: z.record(z.string(), z.string()),
})
export type NotificationWords = z.infer<typeof notificationWordsSchema>

/** A notification to show: its title, and what the system shows it with. */
export interface Shown {
  title: string
  options: NotificationOptions & { data: { conversationId: string } }
}

/**
 * The notification `message` is shown as, in `words`. The tag names its
 * conversation and turn, its request or the agent's notification, so a
 * browser shows each once; the icon is Demi's.
 */
export function notificationOf(message: PushMessage, words: NotificationWords): Shown {
  const title = message.kind === 'agent' ? message.title : (message.title ?? words.untitled)
  const options = (tag: string, body: string): Shown['options'] => ({
    body,
    tag,
    icon: '/icons/icon-192.png',
    data: { conversationId: message.conversationId },
  })
  switch (message.kind) {
    case 'turn_finished':
      return {
        title,
        options: options(
          `turn:${message.conversationId}:${message.turn}`,
          message.answerStart === null ? '' : truncate(markdownPlainText(message.answerStart), ANSWER_START_CHARS),
        ),
      }
    case 'turn_failed':
      return { title, options: options(`turn:${message.conversationId}:${message.turn}`, words.turnStopped) }
    case 'permission':
      return { title, options: options(`permission:${message.request}`, askOf(message.ask, words)) }
    case 'agent':
      return { title, options: options(`agent:${message.id}`, message.body) }
  }
}

/** What a request asks the user, as its card's title asks it. */
function askOf(ask: Extract<PushMessage, { kind: 'permission' }>['ask'], words: NotificationWords): string {
  switch (ask.kind) {
    case 'permission':
      return requestTitleIn(words, ask)
    case 'credential':
      return fillText(words.saveCredential, { name: ask.name })
    case 'allowance':
      return fillText(words.useCredential, { name: ask.name })
  }
}

/**
 * What a push without the page's words says: Demi, and the values the
 * push carries, as on a browser whose words were cleared.
 */
const NO_WORDS: NotificationWords = {
  untitled: 'Demi',
  turnStopped: '',
  saveCredential: '',
  allow: '',
  reach: '',
  pairing: '',
  useCredential: '',
  actions: {},
}

/**
 * The notification a push's `message` shows, in the words `readWords`
 * reads. Every push shows one, as the browser requires: a push that does
 * not read shows Demi, and words that cannot be read, such as when the
 * browser's storage fails, give way to the push's own values.
 */
export async function pushedNotification(
  message: unknown,
  readWords: () => Promise<NotificationWords | null>,
): Promise<{ title: string; options?: NotificationOptions }> {
  const parsed = pushMessageSchema.safeParse(message)
  if (!parsed.success) {
    console.error('A push does not read', parsed.error)
    return { title: 'Demi' }
  }
  let words: NotificationWords | null = null
  try {
    words = await readWords()
  } catch (error) {
    console.error('The notification words could not be read', error)
  }
  return notificationOf(parsed.data, words ?? NO_WORDS)
}

/** A window of the site, as the service worker's clients list it. */
export interface SitePage {
  readonly url: string
  readonly focused: boolean
  readonly visibilityState: DocumentVisibilityState
}

/**
 * The page of the product a click on a notification opens its conversation
 * in: the focused one, else a visible one, else any; never a share's page
 * (`/s/…`), which is no product page. None when no product page is open.
 */
export function productPage<P extends SitePage>(pages: readonly P[]): P | undefined {
  const product = pages.filter((page) => !new URL(page.url).pathname.startsWith('/s/'))
  return product.find((page) => page.focused)
    ?? product.find((page) => page.visibilityState === 'visible')
    ?? product[0]
}
