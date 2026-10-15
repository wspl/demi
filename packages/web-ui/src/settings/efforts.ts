import { upperFirst } from '@demicodes/utils'
import { t } from './locales/text'

/** A reasoning effort as the page names it, in the page's language; one a catalogue does not know shows as its id. */
export function effortLabel(effort: string): string {
  switch (effort) {
    case 'minimal':
      return t('efforts.minimal')
    case 'low':
      return t('efforts.low')
    case 'medium':
      return t('efforts.medium')
    case 'high':
      return t('efforts.high')
    case 'max':
      return t('efforts.max')
    default:
      return upperFirst(effort)
  }
}
