import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The shared page's text. */
export const { t, find } = defineText('ui.share', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
