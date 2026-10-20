import { defineText } from '../../i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The text of `web-ui/src/transport`. */
export const { t } = defineText('ui.transport', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
