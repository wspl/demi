import { z } from 'zod'

/**
 * The languages Demi speaks and how the page chooses one.
 *
 * The page alone resolves System, and reports the language it resolved with the
 * browser's other facts in the `locale` preference, so the backend's emails read
 * that instead of resolving it again.
 *
 * It starts in the language it last showed, kept in the browser's storage, so a
 * user whose choice differs from the browser's sees no other language before the
 * preferences arrive.
 *
 * The document's `lang` follows the choice, so each language's own font draws its
 * characters: one Han character looks different in Chinese, Japanese and Korean.
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
 * The language Demi speaks for the BCP 47 tag `tag`, if any:
 *
 * - Traditional Chinese for `zh-Hant` and the Chinese of Taiwan, Hong Kong and
 *   Macao.
 * - Simplified for any other `zh`.
 * - A script the tag names wins over its region.
 * - A tag that is not one speaks none.
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

const SHOWN_LANGUAGE_KEY = 'demi-language'
const languageSchema = z.enum(LANGUAGES)

/**
 * The language the page last showed in this browser, which it starts in,
 * so a user whose choice differs from the browser's sees no other language
 * before the preferences arrive; null before the first.
 */
export function lastShownLanguage(): Language | null {
  if (typeof localStorage === 'undefined')
    return null
  const kept = languageSchema.safeParse(localStorage.getItem(SHOWN_LANGUAGE_KEY))
  return kept.success ? kept.data : null
}

/** Remembers `language` as the one the page shows, for its next start. */
export function keepShownLanguage(language: Language): void {
  if (typeof localStorage !== 'undefined')
    localStorage.setItem(SHOWN_LANGUAGE_KEY, language)
}
