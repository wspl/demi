import type { Language, LanguageDescription } from '@codemirror/language'
import type { Tree } from '@lezer/common'
import { highlightCode, highlightTree } from '@lezer/highlight'
import { StyleModule } from 'style-mod'
import { appThemeStore } from '../theme/appTheme'
import { fenceLanguage } from '../editor/language'
import { reportSyntaxColorsUnavailable } from '../editor/load'
import { codeHighlightStyle } from '../editor/theme/highlightStyle'
import type { CodeRun } from './highlight'
import { escapeHtml } from './html'

// The highlighter of code outside the editor, in the editor's highlighter and
// theme (`file-previews.md` § Code): its languages' parsers, loaded as a fence
// first names each, and its highlight style's classes. A chunk of its own,
// which `highlight.ts` loads as the first code shows.

/** The highlighter, which tells `languageArrived` each time a language it was asked for has loaded. */
export function createHighlighter(languageArrived: () => void) {
  /** The languages asked for, each loaded once; one that fails to load stays plain text. */
  const requested = new Set<LanguageDescription>()

  /** The loaded language a fence names; null until it has loaded, and for one the highlighter does not know. */
  function loadedLanguage(fence: string): Language | null {
    const description = fenceLanguage(fence)
    if (!description)
      return null
    if (description.support)
      return description.support.language
    if (!requested.has(description)) {
      requested.add(description)
      description.load().then(languageArrived, reportSyntaxColorsUnavailable)
    }
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
    /** Code's text as HTML in the editor's classes: escaped plain text while its language loads, and for one the highlighter does not know. */
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

    /** Code's highlighted runs; none while its language loads, and for one the highlighter does not know. */
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

    /** A language's full name as the highlighter knows it (`ts` is TypeScript); one it does not know is the fence as written. */
    label(fence: string): string {
      return fenceLanguage(fence)?.name ?? fence
    },
  }
}

export type Highlighter = ReturnType<typeof createHighlighter>
