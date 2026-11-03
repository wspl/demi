import { defineText } from '../../i18n/text'
import de from './de.json'
import en from './en.json'
import es419 from './es-419.json'
import fr from './fr.json'
import ja from './ja.json'
import ko from './ko.json'
import ptBR from './pt-BR.json'
import zhHans from './zh-Hans.json'
import zhHant from './zh-Hant.json'

/** The conversation requests' text. */
export const { t, find } = defineText('ui.permissions', { en, de, 'es-419': es419, fr, 'pt-BR': ptBR, 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko })
