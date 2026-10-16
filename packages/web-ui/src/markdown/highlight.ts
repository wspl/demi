import { readonly, shallowRef } from 'vue'
import { appThemeStore } from '../theme/appTheme'
import { codeTheme } from '../theme/codeTheme'
import { reportSyntaxColorsUnavailable } from '../editor/load'
import type { Highlighter } from './highlighter'
import { escapeHtml } from './html'

// Code outside the editor (`file-previews.md` § Code), as the page shows it
// before and after the highlighter arrives. The highlighter is a chunk of its
// own, which the first code shown starts loading: until it is there, code
// shows as plain text in the theme's text color, laid out as it will be, and
// a block's language label keeps its row, empty; on its arrival every code
// renders again with its colors and its label.

const renderVersion = shallowRef(0)

// Code shows in the app's mode, so a mode switch is a new render too. The
// subscription lasts as long as the page, like the loaded languages.
appThemeStore.subscribe(() => {
  renderVersion.value += 1
})

let highlighter: Highlighter | null = null
let loading: Promise<void> | null = null

/**
 * Loads the highlighter once; code renders again when it is there. One that
 * fails to load leaves code plain, as a language that fails to load does.
 */
export function loadHighlighter(): Promise<void> {
  loading ??= import('./highlighter').then(
    ({ createHighlighter }) => {
      highlighter = createHighlighter(() => {
        renderVersion.value += 1
      })
      renderVersion.value += 1
    },
    reportSyntaxColorsUnavailable,
  )
  return loading
}

/** The highlighter, null until it has loaded: the first call starts its load. */
function loadedHighlighter(): Highlighter | null {
  if (!highlighter)
    void loadHighlighter()
  return highlighter
}

/** The color of code's text where no token colors it, in the page's mode. */
export function codeTextColor(): string {
  return codeTheme[appThemeStore.state.mode].palette.foreground
}

/**
 * Code as HTML, in the editor's classes and its text color: plain text in
 * that color while the highlighter or its language loads, and for one the
 * highlighter does not know.
 */
export function codeToHtml(code: string, lang: string): string {
  const html = loadedHighlighter()?.html(code, lang) ?? escapeHtml(code)
  return `<pre style="color: ${codeTextColor()}" tabindex="0"><code>${html}</code></pre>`
}

/**
 * A fenced block as a message or a document shows it: highlighted, under the
 * full name of the language its fence names, when it names one; the label's
 * row is there, empty, while the highlighter loads. The frame is where the
 * page puts its Copy (`CodeBlockCopy.vue`), level with the language.
 */
export function codeBlockHtml(code: string, lang: string): string {
  const fence = lang.trim().split(/\s/, 1)[0] ?? ''
  // The code first: its render starts the highlighter's load.
  const pre = codeToHtml(code, fence)
  const label = fence ? `<div class="code-block-lang">${escapeHtml(highlighter?.label(fence) ?? '')}</div>` : ''
  return `<div class="code-block">${label}${pre}</div>`
}

/** A highlighted run of code: where it starts and ends in the code, and the classes that color it. */
export interface CodeRun {
  from: number
  to: number
  classes: string
}

/**
 * Code's highlighted runs in the page's mode, as a code block colors them;
 * the text between them is in `codeTextColor`. None while the highlighter or
 * its language loads, and for one the highlighter does not know.
 */
export function codeRuns(code: string, lang: string): CodeRun[] {
  return loadedHighlighter()?.runs(code, lang) ?? []
}

/** Changes when rendered code should render again: the highlighter's or a language's arrival, and the page's mode. */
export function useMarkdownRenderVersion() {
  return readonly(renderVersion)
}
