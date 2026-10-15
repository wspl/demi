import { definePluginText } from '@demicodes/plugin-sdk'
import { PLUGIN } from '../generated/plugin'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The skills plugin's own text (`localization.md` § Where the text lives). */
export const { t } = definePluginText(PLUGIN, { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
