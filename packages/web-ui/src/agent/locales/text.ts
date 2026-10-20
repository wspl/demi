import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The text of the agent area: the transcript, composer, dock, windows and menus. */
export const { t, find } = defineText('ui.agent', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })

/** Marks where something that is not text stands in a sentence, which `around` splits at. */
const PLACE = '\u0000'

/**
 * The sentence at `key` split around `place`, the name of its one value that is not text,
 * such as an agent's link: what comes before it and what comes after, each as the
 * language writes it, so a language that puts the value first is worded as it reads.
 */
export function around(key: Parameters<typeof t>[0], place: string, named: Record<string, string> = {}): [before: string, after: string] {
  const [before = '', after = ''] = t(key, { ...named, [place]: PLACE }).split(PLACE)
  return [before, after]
}
