import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The conversation requests' text. */
export const { t, find } = defineText('ui.permissions', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
