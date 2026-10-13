/** A background of an avatar: a vertical gradient from its top color to its bottom one, in each color scheme. */
export type AvatarTone = {
  light: readonly [top: string, bottom: string]
  dark: readonly [top: string, bottom: string]
}

/** The macOS Contacts gray, the one tone of the Contacts scheme. */
export const CONTACTS_TONE: AvatarTone = {
  light: ['#a5abb8', '#858994'],
  dark: ['#7b8090', '#5a5e6b'],
}

/**
 * Eight muted, desaturated tones that tell accounts apart in a list; none
 * is a saturated blue or purple. White letters read on every one.
 */
export const HUE_TONES: readonly AvatarTone[] = [
  { light: ['#858fa1', '#667084'], dark: ['#667082', '#4a5262'] }, // slate
  { light: ['#86a086', '#678267'], dark: ['#62785f', '#485b49'] }, // sage
  { light: ['#b8826b', '#96634e'], dark: ['#8e6654', '#6f4b3c'] }, // clay
  { light: ['#b09a68', '#8c7646'], dark: ['#8a7650', '#6c5a3a'] }, // sand
  { light: ['#9188a4', '#71688a'], dark: ['#6f677e', '#534c61'] }, // dusk
  { light: ['#8a9560', '#6b7640'], dark: ['#6b7447', '#515936'] }, // moss
  { light: ['#a07e8f', '#7e5c6d'], dark: ['#7b5b6a', '#5e4352'] }, // plum
  { light: ['#7592ab', '#567289'], dark: ['#587087', '#41566b'] }, // steel
]

/** The tone an account id always gets: its FNV-1a hash picks one of the eight. */
export function hueToneOf(accountId: string): AvatarTone {
  let hash = 0x811c9dc5
  for (const unit of accountId) {
    hash = Math.imul(hash ^ unit.codePointAt(0)!, 0x01000193) >>> 0
  }
  return HUE_TONES[hash % HUE_TONES.length]!
}
