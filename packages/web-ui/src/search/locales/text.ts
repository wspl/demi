import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The search area's text. */
export const { t, find } = defineText('ui.search', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
