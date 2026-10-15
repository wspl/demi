import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The files folder's own text (`localization.md` § Where the text lives). */
export const { t } = defineText('ui.files', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
