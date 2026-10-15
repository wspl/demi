import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The sign-in pages and their steps' text (`localization.md` § Where the text lives). */
export const { t, find } = defineText('ui.auth', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
