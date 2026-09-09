import type { SettingsProviderEntry, SettingsVendor } from './types'

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
