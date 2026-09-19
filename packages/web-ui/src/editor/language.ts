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
import { baseName } from '../files/paths'

const nameOverrides: Record<string, string> = {
  Dockerfile: 'Dockerfile',
  Makefile: 'Makefile',
  '.gitignore': 'Shell',
}

// The common languages ship with the editor; the rest load from language-data on demand.
const bundledLanguages: readonly { name: RegExp; load: () => Extension }[] = [
  { name: /\.tsx$/i, load: () => javascript({ typescript: true, jsx: true }) },
  { name: /\.ts$/i, load: () => javascript({ typescript: true }) },
  { name: /\.jsx$/i, load: () => javascript({ jsx: true }) },
  { name: /\.(mjs|cjs|js)$/i, load: () => javascript() },
  { name: /\.(json|jsonc)$/i, load: () => json() },
  { name: /\.(md|mdx)$/i, load: () => markdown() },
  { name: /\.vue$/i, load: () => vue() },
  { name: /\.html$/i, load: () => html() },
  { name: /\.(css|scss|sass|less)$/i, load: () => css() },
]

/** The language a file's name selects, if any. */
function findLanguage(name: string): (() => Promise<Extension>) | null {
  const bundled = bundledLanguages.find((language) => language.name.test(name))
  if (bundled)
    return async () => bundled.load()
  const override = nameOverrides[name]
  const description = override
    ? LanguageDescription.matchLanguageName(languages, override)
    : LanguageDescription.matchFilename(languages, name)
  return description ? () => description.load() : null
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
    reportError('Syntax colors are unavailable', error)
    return []
  }
}
