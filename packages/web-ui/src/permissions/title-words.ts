import { fillText } from '@demicodes/utils'

/**
 * A request's card title worded from words given whole, with no catalogue
 * running: the page words its cards so, and its service worker a request's
 * notification. The module imports nothing that needs a page.
 */

/**
 * What the titles of the requests' cards say in the page's language, the title
 * left as `{action}`, with the action of each category by its catalogue key,
 * its id with `_` for `.`.
 *
 * It is what a card's title is worded from, and what the page hands its service
 * worker, which words a request's notification while no page runs.
 */
export interface RequestTitleWords {
  allow: string
  actions: Record<string, string>
  /** A device's request's title, the device left as `{name}`. */
  useDevice: string
}

/** The catalogue key of the category `id`. */
export function categoryKey(id: string): string {
  return id.replaceAll('.', '_')
}

/**
 * The card's title in `words`: its category's action asked, its declared
 * action or its id where `words` has none.
 */
export function requestTitleIn(
  words: RequestTitleWords,
  request: { categories: readonly { id: string; action?: string | null }[] },
): string {
  // A request asks for its one category.
  const category = request.categories[0]!
  return fillText(words.allow, { action: words.actions[categoryKey(category.id)] ?? category.action ?? category.id })
}

/** A device's request's title in `words`, for the device `name`. */
export function deviceTitleIn(words: RequestTitleWords, name: string): string {
  return fillText(words.useDevice, { name })
}

