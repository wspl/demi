import type { Language, LanguageDescription } from '@codemirror/language'
import type { Tree } from '@lezer/common'
import { highlightCode, highlightTree } from '@lezer/highlight'
import { StyleModule } from 'style-mod'
import { appThemeStore } from '../theme/appTheme'
import { regionsChanged } from '../infra/region-wait'
import { fenceLanguage } from '../editor/language'
import { reportSyntaxColorsUnavailable } from '../editor/load'
import { codeHighlightStyle } from '../editor/theme/highlightStyle'
import { COMMON_FENCES } from './code-labels'
import type { CodeRun } from './highlight'
import { escapeHtml } from './html'

// The highlighter of code outside the editor, in the editor's highlighter and
// theme: its languages' parsers, loaded as a fence first names each (the common
// ones in the background, once the first conversation has shown), and its
// highlight style's classes. A chunk of its own, which `highlight.ts` loads.

/**
 * The highlighter, which tells `languageArrived` each time a language it was
 * asked for has loaded.
 */
export function createHighlighter(languageArrived: () => void) {
  /**
   * The languages asked for, each loaded once; one that fails to load stays
   * plain text.
   */
  const requested = new Map<LanguageDescription, Promise<void>>()

  /** Starts a language's load once. */
  function request(description: LanguageDescription): Promise<void> {
    const loading = requested.get(description)
    if (loading)
      return loading
    const load = description.load().then(() => {
      regionsChanged()
      languageArrived()
    }, reportSyntaxColorsUnavailable)
    requested.set(description, load)
    return load
  }

  /**
   * The loaded language a fence names; null until it has loaded, and for one
   * the highlighter does not know.
   */
  function loadedLanguage(fence: string): Language | null {
    const description = fenceLanguage(fence)
    if (!description)
      return null
    if (description.support)
      return description.support.language
    void request(description)
    return null
  }

  /**
   * The parses of the code most recently shown, by language and text, so a
   * streaming message parses only the block whose text grew, and a render for
   * a mode switch or another block parses nothing.
   */
  const parses = new Map<string, Tree>()
  const PARSES_KEPT = 128

  function parsed(language: Language, code: string): Tree {
    const key = `${language.name}\n${code}`
    const kept = parses.get(key)
    if (kept) {
      // The most recently used goes last, so the oldest is the first to go.
      parses.delete(key)
      parses.set(key, kept)
      return kept
    }
    const tree = language.parser.parse(code)
    parses.set(key, tree)
    if (parses.size > PARSES_KEPT)
      parses.delete(parses.keys().next().value!)
    return tree
  }

  let stylesMounted = false

  /**
   * The highlight style for the page's mode, with the CSS of both modes'
   * classes in the page, as an editor puts its own there. A render without a
   * page, as in a test, has nowhere to put them and needs none.
   */
  function pageHighlightStyle() {
    if (!stylesMounted && typeof document !== 'undefined') {
      StyleModule.mount(document, [codeHighlightStyle('dark').module, codeHighlightStyle('light').module].filter((module) => module !== null))
      stylesMounted = true
    }
    return codeHighlightStyle(appThemeStore.state.mode)
  }

  return {
    /**
     * Code's text as HTML in the editor's classes: escaped plain text while its
     * language loads, and for one the highlighter does not know.
     */
    html(code: string, fence: string): string {
      const language = loadedLanguage(fence)
      if (!language)
        return escapeHtml(code)
      let html = ''
      highlightCode(code, parsed(language, code), pageHighlightStyle(), (text, classes) => {
        html += classes ? `<span class="${classes}">${escapeHtml(text)}</span>` : escapeHtml(text)
      }, () => {
        html += '\n'
      })
      return html
    },

    /**
     * Whether the code of a fence has all it needs of the highlighter: its
     * language is in, or it names none the highlighter knows.
     */
    languageReady(fence: string): boolean {
      const description = fenceLanguage(fence)
      return description === null || description.support !== undefined
    },

    /** Loads the common languages, each once, and resolves when they are in. */
    async loadCommonLanguages(): Promise<void> {
      const descriptions = COMMON_FENCES.flatMap((fence) => fenceLanguage(fence) ?? [])
      await Promise.all(descriptions.map((description) => description.support ? undefined : request(description)))
    },

    /**
     * Code's highlighted runs; none while its language loads, and for one the
     * highlighter does not know.
     */
    runs(code: string, fence: string): CodeRun[] {
      const language = loadedLanguage(fence)
      if (!language)
        return []
      const runs: CodeRun[] = []
      highlightTree(parsed(language, code), pageHighlightStyle(), (from, to, classes) => {
        runs.push({ from, to, classes })
      })
      return runs
    },
  }
}

export type Highlighter = ReturnType<typeof createHighlighter>
