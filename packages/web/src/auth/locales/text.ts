import { defineText } from '@demicodes/web-ui/i18n/text'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The text of `web/src/auth`. */
export const { t } = defineText('web.auth', { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
