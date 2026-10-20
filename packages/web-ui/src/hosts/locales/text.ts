import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The hosts area's text. */
export const { t, find } = defineText('ui.hosts', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
