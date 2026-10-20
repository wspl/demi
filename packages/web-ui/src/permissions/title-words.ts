import { fillText } from '@demicodes/utils'

/**
 * A request's card title worded from words given whole, with no catalogue
 * running: the page words its cards so, and its service worker a request's
 * notification. The module imports nothing that needs a page.
 */

/**
 * What the titles of the requests' cards say in the page's language, each value left
 * as `{action}`, `{host}` or `{name}`, with the action of each category by its
 * catalogue key, its id with `_` for `.`.
 *
 * It is what a card's title is worded from, and what the page hands its service
 * worker, which words a request's notification while no page runs.
 */
export interface RequestTitleWords {
  allow: string
  reach: string
  pairing: string
  useCredential: string
  actions: Record<string, string>
}

/** The catalogue key of the category `id`. */
export function categoryKey(id: string): string {
  return id.replaceAll('.', '_')
}

/**
 * The card's title in `words`: a host's asks to reach it, a category's to
 * do its action, its declared action or its id where `words` has none.
 */
export function requestTitleIn(
  words: RequestTitleWords,
  request: {
    categories: readonly { id: string; action?: string | null }[]
    host: { kind: 'device'; name: string } | { kind: 'pairing' } | null
  },
): string {
  if (request.host) {
    return request.host.kind === 'device' ? fillText(words.reach, { host: request.host.name }) : words.pairing
  }
  // A request asks for its host or, without one, for its one category.
  const category = request.categories[0]!
  return fillText(words.allow, { action: words.actions[categoryKey(category.id)] ?? category.action ?? category.id })
}

