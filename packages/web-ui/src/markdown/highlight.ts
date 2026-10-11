import { readonly, shallowRef } from 'vue'
import type { Language, LanguageDescription } from '@codemirror/language'
import type { Tree } from '@lezer/common'
import { highlightCode, highlightTree } from '@lezer/highlight'
import { StyleModule } from 'style-mod'
import { reportError } from '../infra/errors'
import { appThemeStore } from '../theme/appTheme'
import { codeTheme } from '../theme/codeTheme'
import { fenceLanguage } from '../editor/language'
import { codeHighlightStyle } from '../editor/theme/highlightStyle'
import { escapeHtml } from './html'

// Code outside the editor, in the editor's highlighter and theme
// (`file-previews.md` § Code): its languages' parsers, loaded as a fence
// first names each, and its highlight style's classes.

const renderVersion = shallowRef(0)

// Code shows in the app's mode, so a mode switch is a new render too. The
// subscription lasts as long as the page, like the loaded languages.
appThemeStore.subscribe(() => {
  renderVersion.value += 1
})

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
    description.load().then(
      () => {
        renderVersion.value += 1
      },
      (error: unknown) => reportError('Syntax Colors Are Unavailable', error),
    )
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

/** The color of code's text where no token colors it, in the page's mode. */
export function codeTextColor(): string {
  return codeTheme[appThemeStore.state.mode].palette.foreground
}

/**
 * Code as HTML, in the editor's classes and its text color: plain text in
 * that color while its language loads, and for one the highlighter does
 * not know.
 */
export function codeToHtml(code: string, lang: string): string {
  const language = loadedLanguage(lang)
  let html = ''
  if (language) {
    highlightCode(code, parsed(language, code), pageHighlightStyle(), (text, classes) => {
      html += classes ? `<span class="${classes}">${escapeHtml(text)}</span>` : escapeHtml(text)
    }, () => {
      html += '\n'
    })
  } else {
    html = escapeHtml(code)
  }
  return `<pre style="color: ${codeTextColor()}" tabindex="0"><code>${html}</code></pre>`
}

/** A language's full name as the highlighter knows it (`ts` is TypeScript); one it does not know shows as the fence wrote it. */
function languageLabel(fence: string): string {
  return fenceLanguage(fence)?.name ?? fence
}

/**
 * A fenced block as a message or a document shows it: highlighted, under the
 * full name of the language its fence names, when it names one. The frame is
 * where the page puts its Copy (`CodeBlockCopy.vue`), level with the language.
 */
export function codeBlockHtml(code: string, lang: string): string {
  const fence = lang.trim().split(/\s/, 1)[0] ?? ''
  const label = fence ? `<div class="code-block-lang">${escapeHtml(languageLabel(fence))}</div>` : ''
  return `<div class="code-block">${label}${codeToHtml(code, fence)}</div>`
}

/** A highlighted run of code: where it starts and ends in the code, and the classes that color it. */
export interface CodeRun {
  from: number
  to: number
  classes: string
}

/**
 * Code's highlighted runs in the page's mode, as a code block colors them;
 * the text between them is in `codeTextColor`. None while its language
 * loads, and for one the highlighter does not know.
 */
export function codeRuns(code: string, lang: string): CodeRun[] {
  const language = loadedLanguage(lang)
  if (!language)
    return []
  const runs: CodeRun[] = []
  highlightTree(parsed(language, code), pageHighlightStyle(), (from, to, classes) => {
    runs.push({ from, to, classes })
  })
  return runs
}

/** Changes when rendered code should render again: a language's arrival and the page's mode. */
export function useMarkdownRenderVersion() {
  return readonly(renderVersion)
}
