import type {
  SettingsProviderEntry,
  SettingsVendor,
  SettingsWireApi,
} from './types'

const OFFICIAL_VENDOR_BY_PROTOCOL: Record<SettingsWireApi, string> = {
  'anthropic-messages': 'anthropic',
  'openai-responses': 'openai',
  'openai-chat': 'openai',
  'google-generative': 'google',
}

const DEFAULT_API_VENDORS = ['openai', 'anthropic'] as const

/** Offer common vendors once, using the host's catalog for endpoint metadata. */
export function defaultApiVendors(
  vendors: readonly SettingsVendor[],
  providers: readonly Pick<SettingsProviderEntry, 'vendorId'>[],
): SettingsVendor[] {
  return DEFAULT_API_VENDORS.flatMap((id) => {
    const vendor = vendors.find((entry) => entry.id === id)
    return vendor && !providers.some((entry) => entry.vendorId === id)
      ? [vendor]
      : []
  })
}

/** Bare protocol entries start from the official vendor's catalog endpoint. */
export function defaultEndpointUrl(
  vendors: readonly SettingsVendor[],
  wireApi: SettingsWireApi,
): string {
  const vendorId = OFFICIAL_VENDOR_BY_PROTOCOL[wireApi]
  return vendors.find((vendor) => vendor.id === vendorId)?.baseUrl ?? ''
}
