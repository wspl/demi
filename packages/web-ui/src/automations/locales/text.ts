import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The Automations page's text (`localization.md` § Where the text lives). */
export const { t, find } = defineText('ui.automations', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
