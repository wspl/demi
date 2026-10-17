import type { ImageVendorId, SettingsProviderEntry } from './types'

/**
 * Settings › Agent's Image Generation rows (`images.md` § Which entry
 * makes an image): one for each vendor an entry of the user makes images
 * for, with the vendor's entries the user may choose and the one Automatic
 * takes now, which the backend answers.
 */
export interface ImageVendorRow {
  vendor: ImageVendorId
  /** How the user knows the vendor. */
  name: string
  /** The entries whose Image Generation is on, in the order of Settings › Providers. */
  entries: ImageEntryOption[]
  /** The entry Automatic takes now; null when none can. */
  automatic: ImageEntryOption | null
}

export interface ImageEntryOption {
  id: string
  /** The entry's name with its account, as Small Model names a model's entry: `ChatGPT · alex@example.com`. */
  label: string
}

/** The vendors in the order of their rows, with their names. */
const VENDORS: readonly (readonly [ImageVendorId, string])[] = [
  ['openai', 'OpenAI'],
  ['google', 'Google'],
  ['xai', 'xAI'],
]

/** `entry` as a row names it: its name, and its active account's label when it has one. */
function option(entry: SettingsProviderEntry): ImageEntryOption {
  const account = entry.accounts.find((candidate) => candidate.active)?.label
  return { id: entry.id, label: account ? `${entry.name} · ${account}` : entry.name }
}

/**
 * The rows for `entries`, one for each vendor `automatic` names: the
 * backend names every vendor an entry makes images for, with the entry
 * Automatic takes for it or null.
 */
export function imageVendorRows(
  entries: readonly SettingsProviderEntry[],
  automatic: Partial<Record<ImageVendorId, string | null>>,
): ImageVendorRow[] {
  return VENDORS.flatMap(([vendor, name]) => {
    if (!(vendor in automatic)) {
      return []
    }
    const own = entries.filter((entry) => entry.imageVendor === vendor && entry.enabled)
    const taken = own.find((entry) => entry.id === automatic[vendor])
    return [{
      vendor,
      name,
      entries: own.filter((entry) => entry.imageGeneration).map(option),
      automatic: taken ? option(taken) : null,
    }]
  })
}
