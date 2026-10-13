/** A background of an avatar: a vertical gradient from its top color to its bottom one, in each color scheme. */
export type AvatarTone = {
  name: string
  light: readonly [top: string, bottom: string]
  dark: readonly [top: string, bottom: string]
}

/**
 * Eight muted, desaturated tones that tell accounts apart in a list; none is
 * a saturated blue or purple. White letters read on every one: at the height
 * where the letters of a 24 px avatar sit (31% down the gradient, the top edge
 * of the capitals) the contrast is at least 4.5:1 in both color schemes, the
 * WCAG bar for normal text, since 11 px letters are not large text. The
 * circle's own top edge, which carries no letter, is lighter.
 */
export const HUE_TONES: readonly AvatarTone[] = [
  { name: 'slate', light: ['#757e8e', '#5a6374'], dark: ['#667082', '#4a5262'] },
  { name: 'sage', light: ['#6d826d', '#536953'], dark: ['#62785f', '#485b49'] },
  { name: 'clay', light: ['#a2725e', '#845745'], dark: ['#8e6654', '#6f4b3c'] },
  { name: 'sand', light: ['#8e7c54', '#715f38'], dark: ['#8a7650', '#6c5a3a'] },
  { name: 'dusk', light: ['#827a93', '#655d7c'], dark: ['#6f677e', '#534c61'] },
  { name: 'moss', light: ['#788254', '#5d6738'], dark: ['#6b7447', '#515936'] },
  { name: 'plum', light: ['#967686', '#765666'], dark: ['#7b5b6a', '#5e4352'] },
  { name: 'steel', light: ['#678096', '#4c6479'], dark: ['#587087', '#41566b'] },
]

/** The tone an account id always gets: its FNV-1a hash picks one of the eight. */
export function hueToneOf(accountId: string): AvatarTone {
  let hash = 0x811c9dc5
  for (const unit of accountId) {
    hash = Math.imul(hash ^ unit.codePointAt(0)!, 0x01000193) >>> 0
  }
  return HUE_TONES[hash % HUE_TONES.length]!
}
