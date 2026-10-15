import { t } from './locales/text'
import type { SentenceText } from '../ui/ui-text'
import type { SettingsVendor } from './types'

/** The vendors Add Provider lists first, in this order, before the rest of the catalog. */
const FIRST_VENDORS = ['openai', 'anthropic', 'google']

/** The catalog's vendors in the order Add Provider lists them: the common ones first, then the rest as they come. */
export function vendorsInListOrder(vendors: readonly SettingsVendor[]): SettingsVendor[] {
  const first = FIRST_VENDORS.flatMap((id) => vendors.filter((vendor) => vendor.id === id))
  return [...first, ...vendors.filter((vendor) => !FIRST_VENDORS.includes(vendor.id))]
}

/** What is wrong with an endpoint the user typed, said as the fix; null for an http or https address. */
export function endpointProblem(text: string): SentenceText | null {
  try {
    const { protocol } = new URL(text.trim())
    if (protocol === 'http:' || protocol === 'https:') {
      return null
    }
  } catch {
    // Not an address at all: said below with the rest.
  }
  return t('provider.endpointInvalid')
}
