import type { Extension } from '@codemirror/state'
import { LanguageDescription } from '@codemirror/language'
import { languages } from '@codemirror/language-data'
import { css } from '@codemirror/lang-css'
import { html } from '@codemirror/lang-html'
import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { markdown } from '@codemirror/lang-markdown'
import { vue } from '@codemirror/lang-vue'
import { reportError } from '../infra/errors'
import { t } from './locales/text'
import { baseName } from '@demicodes/utils'

/** Files whose name the language data does not match, by the language-data name they read as. */
const nameOverrides: Record<string, string> = {
  '.gitignore': 'Shell',
}

// The common languages ship with the editor; the rest load from language-data on demand. Each names
// the language-data language it reads as, for the files the data itself does not match, such as `.jsonc`.
const bundledLanguages: readonly { name: RegExp; language: string; load: () => Extension }[] = [
  { name: /\.tsx$/i, language: 'TSX', load: () => javascript({ typescript: true, jsx: true }) },
  { name: /\.ts$/i, language: 'TypeScript', load: () => javascript({ typescript: true }) },
  { name: /\.jsx$/i, language: 'JSX', load: () => javascript({ jsx: true }) },
  { name: /\.(mjs|cjs|js)$/i, language: 'JavaScript', load: () => javascript() },
  { name: /\.(json|jsonc)$/i, language: 'JSON', load: () => json() },
  { name: /\.(md|mdx)$/i, language: 'Markdown', load: () => markdown() },
  { name: /\.vue$/i, language: 'Vue', load: () => vue() },
  { name: /\.html$/i, language: 'HTML', load: () => html() },
  { name: /\.(css|scss|sass|less)$/i, language: 'CSS', load: () => css() },
]

/**
 * The language a file's name selects, as the language data describes it:
 * the one owner of "the language of this name", for a file's colors, the
 * name the File view's bar gives it, and a code fence that names its
 * files' extension. Null for a name no language matches.
 */
function nameLanguage(name: string): LanguageDescription | null {
  const override = nameOverrides[name]
  if (override)
    return LanguageDescription.matchLanguageName(languages, override)
  const bundled = bundledLanguages.find((language) => language.name.test(name))
  return LanguageDescription.matchFilename(languages, name)
    ?? (bundled ? LanguageDescription.matchLanguageName(languages, bundled.language) : null)
}

/** The language a file's name selects, if any: one the editor ships, else the language data's. */
function findLanguage(name: string): (() => Promise<Extension>) | null {
  const bundled = bundledLanguages.find((language) => language.name.test(name))
  if (bundled)
    return async () => bundled.load()
  const description = nameLanguage(name)
  return description ? () => description.load() : null
}

/**
 * The full name of the language a file's text is colored as, by its name,
 * as the language data names it: `TypeScript`, `Markdown`; null for a file
 * the editor colors as no language (`file-previews.md` § The File view's
 * bars).
 */
export function languageName(path: string): string | null {
  return nameLanguage(baseName(path))?.name ?? null
}

/**
 * The language a code fence names (`file-previews.md` § Code), by its name
 * or one of its short names (`typescript`, `ts`) or else by its files'
 * extension (`rs`); null for one the highlighter does not know, which shows
 * as plain text.
 */
export function fenceLanguage(fence: string): LanguageDescription | null {
  // A fence of plain text names no language, as on GitHub, though LaTeX
  // counts `.text` among its files' extensions.
  if (!fence || /^(text|txt|plain|plaintext)$/i.test(fence))
    return null
  return LanguageDescription.matchLanguageName(languages, fence, false)
    ?? nameLanguage(`code.${fence.toLowerCase()}`)
}

/**
 * Syntax colors for a file's text, by its name: empty when no language
 * matches it or its language fails to load.
 */
export async function loadFileLanguage(path: string): Promise<Extension> {
  const load = findLanguage(baseName(path))
  if (!load)
    return []
  try {
    return await load()
  } catch (error) {
    reportSyntaxColorsUnavailable(error)
    return []
  }
}

/** Tells that a language's syntax colors could not load, as the editor and a message's code blocks both do. */
export function reportSyntaxColorsUnavailable(error: unknown): void {
  reportError(t('syntaxColorsUnavailable'), error)
}
