import type { Extension } from '@codemirror/state'
import { LanguageDescription } from '@codemirror/language'
import { languages } from '@codemirror/language-data'
import { css } from '@codemirror/lang-css'
import { html } from '@codemirror/lang-html'
import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { markdown } from '@codemirror/lang-markdown'
import { vue } from '@codemirror/lang-vue'

interface ResolvedLanguage {
  load(): Promise<Extension>
}

const filenameOverrides: Record<string, string> = {
  Dockerfile: 'Dockerfile',
  Makefile: 'Makefile',
  '.gitignore': 'Shell',
}

const directLanguageLoaders: Array<{
  test: (filename: string, basename: string) => boolean
  load: () => Promise<Extension>
}> = [
  {
    test: (filename) => /\.(tsx)$/i.test(filename),
    load: async () => javascript({ typescript: true, jsx: true }),
  },
  {
    test: (filename) => /\.(ts)$/i.test(filename),
    load: async () => javascript({ typescript: true }),
  },
  {
    test: (filename) => /\.(jsx)$/i.test(filename),
    load: async () => javascript({ jsx: true }),
  },
  {
    test: (filename) => /\.(mjs|cjs|js)$/i.test(filename),
    load: async () => javascript(),
  },
  {
    test: (filename) => /\.(json|jsonc)$/i.test(filename),
    load: async () => json(),
  },
  {
    test: (filename) => /\.(md|mdx)$/i.test(filename),
    load: async () => markdown(),
  },
  {
    test: (filename) => /\.(vue)$/i.test(filename),
    load: async () => vue(),
  },
  {
    test: (filename) => /\.(html)$/i.test(filename),
    load: async () => html(),
  },
  {
    test: (filename) => /\.(css|scss|sass|less)$/i.test(filename),
    load: async () => css(),
  },
]

export function findLanguage(filename: string): ResolvedLanguage | null {
  const basename = filename.includes('/') ? filename.slice(filename.lastIndexOf('/') + 1) : filename
  const direct = directLanguageLoaders.find((loader) => loader.test(filename, basename))
  if (direct) return { load: direct.load }

  const override = filenameOverrides[basename]
  const descriptions = languages as unknown as readonly LanguageDescription[]
  const description = override
    ? LanguageDescription.matchLanguageName(descriptions, override)
    : LanguageDescription.matchFilename(descriptions, basename)

  if (!description) return null

  return {
    async load() {
      const language = await description.load()
      return 'extension' in language ? language.extension : language
    },
  }
}
