import { defineText } from '@demicodes/web-ui/i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The gallery's own text: its navigation, views and controls (`localization.md` § Where the text lives). */
export const { t } = defineText('gallery', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
