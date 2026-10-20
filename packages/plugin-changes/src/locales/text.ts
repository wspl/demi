import { definePluginText } from '@demicodes/plugin-sdk'
import { PLUGIN } from '../generated/plugin'
import en from './en.json'
import ja from './ja.json'
import ko from './ko.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The changes plugin's own text. */
export const { t } = definePluginText(PLUGIN, { en, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
