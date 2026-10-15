/**
 * The languages Demi speaks and how the page chooses one
 * (`localization.md` § Choosing the language).
 */

export const LANGUAGES = ['en', 'zh-Hans', 'zh-Hant', 'ja', 'ko'] as const

/** A language Demi speaks, by its BCP 47 tag, as the `language` preference and the catalogues name it. */
export type Language = (typeof LANGUAGES)[number]

/** What the user chose in Settings › General: a language, or the system's. */
export type LanguageChoice = 'system' | Language

/** Each language named in its own language, as the language menu lists it. */
export const LANGUAGE_NAMES: Readonly<Record<Language, string>> = {
  'en': 'English',
  'zh-Hans': '简体中文',
  'zh-Hant': '繁體中文',
  'ja': '日本語',
  'ko': '한국어',
}

/** The regions whose Chinese is written in Traditional characters. */
const TRADITIONAL_REGIONS = new Set(['TW', 'HK', 'MO'])

/**
 * The language Demi speaks for the BCP 47 tag `tag`, if any: Traditional
 * Chinese for `zh-Hant` and the Chinese of Taiwan, Hong Kong and Macao,
 * Simplified for any other `zh`; a script the tag names wins over its
 * region. A tag that is not one speaks none.
 */
function spokenFor(tag: string): Language | null {
  let locale: Intl.Locale
  try {
    locale = new Intl.Locale(tag)
  } catch {
    return null
  }
  switch (locale.language) {
    case 'en':
    case 'ja':
    case 'ko':
      return locale.language
    case 'zh':
      if (locale.script !== undefined) {
        return locale.script === 'Hant' ? 'zh-Hant' : 'zh-Hans'
      }
      return locale.region !== undefined && TRADITIONAL_REGIONS.has(locale.region) ? 'zh-Hant' : 'zh-Hans'
    default:
      return null
  }
}

/**
 * The language `choice` speaks: itself, or for `system` the first of the
 * browser's languages `browser` that Demi speaks, else English.
 */
export function resolveLanguage(choice: LanguageChoice, browser: readonly string[]): Language {
  if (choice !== 'system') {
    return choice
  }
  for (const tag of browser) {
    const spoken = spokenFor(tag)
    if (spoken !== null) {
      return spoken
    }
  }
  return 'en'
}
