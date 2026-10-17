import type { ImageVendorId, SettingsProviderEntry } from './types'

/**
 * Settings › Agent's Image Generation rows (`images.md` § Which entry
 * makes an image): one for each vendor Demi makes images with, always all
 * three, with the vendor's entries the user may choose and the one
 * Automatic takes now, which the backend answers.
 */
export interface ImageVendorRow {
  vendor: ImageVendorId
  /** How the user knows the vendor. */
  name: string
  /** The entries whose Image Generation is on, in the order of Settings › Providers; none makes the vendor not available. */
  entries: ImageEntryOption[]
  /** The entry Automatic takes now; null when none can. */
  automatic: ImageEntryOption | null
}

export interface ImageEntryOption {
  id: string
  /** The entry's name: `Codex`. */
  name: string
  /** Its active account's label, which a menu shows after the name: `name@example.com`; null for an entry without one. */
  account: string | null
}

/** The vendors in the order of their rows, with their names. */
const VENDORS: readonly (readonly [ImageVendorId, string])[] = [
  ['openai', 'OpenAI'],
  ['google', 'Google'],
  ['xai', 'xAI'],
]

/** `entry` as a row names it: its name, and its active account's label when it has one. */
function option(entry: SettingsProviderEntry): ImageEntryOption {
  const account = entry.accounts.find((candidate) => candidate.active)?.label ?? null
  return { id: entry.id, name: entry.name, account }
}

/**
 * The rows for `entries`, one for each vendor: `automatic` names the entry
 * Automatic takes for a vendor, as the backend answers it; a vendor it
 * names none for, or leaves out, has none.
 */
export function imageVendorRows(
  entries: readonly SettingsProviderEntry[],
  automatic: Partial<Record<ImageVendorId, string | null>>,
): ImageVendorRow[] {
  return VENDORS.map(([vendor, name]) => {
    const own = entries.filter((entry) => entry.imageVendor === vendor && entry.enabled)
    const taken = own.find((entry) => entry.id === automatic[vendor])
    return {
      vendor,
      name,
      entries: own.filter((entry) => entry.imageGeneration).map(option),
      automatic: taken ? option(taken) : null,
    }
  })
}
