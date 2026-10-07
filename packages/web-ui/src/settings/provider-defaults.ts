import type { SettingsVendor, SettingsWireApi } from './types'

const OFFICIAL_VENDOR_BY_PROTOCOL: Record<SettingsWireApi, string> = {
  'anthropic-messages': 'anthropic',
  'openai-responses': 'openai',
  'openai-chat': 'openai',
  'google-generative': 'google',
}

/** The vendors Add Provider lists first, in this order, before the rest of the catalog. */
const FIRST_VENDORS = ['openai', 'anthropic', 'google']

/** The catalog's vendors in the order Add Provider lists them: the common ones first, then the rest as they come. */
export function vendorsInListOrder(vendors: readonly SettingsVendor[]): SettingsVendor[] {
  const first = FIRST_VENDORS.flatMap((id) => vendors.filter((vendor) => vendor.id === id))
  return [...first, ...vendors.filter((vendor) => !FIRST_VENDORS.includes(vendor.id))]
}

/** Bare protocol entries start from the official vendor's catalog endpoint. */
export function defaultEndpointUrl(
  vendors: readonly SettingsVendor[],
  wireApi: SettingsWireApi,
): string {
  const vendorId = OFFICIAL_VENDOR_BY_PROTOCOL[wireApi]
  return vendors.find((vendor) => vendor.id === vendorId)?.baseUrl ?? ''
}
