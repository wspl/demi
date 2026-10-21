import { appThemeStore } from '../theme/appTheme'
import { codeTheme } from '../theme/codeTheme'
import { reportSyntaxColorsUnavailable } from '../editor/load'
import { regionForm, regionsChanged, type RegionForm } from '../infra/region-wait'
import { fenceLabel } from './code-labels'
import type { Highlighter } from './highlighter'
import { escapeHtml } from './html'
import { renderAgain } from './render-version'

// Code outside the editor, wherever it shows: a reply, a message, a command's
// row, a terminal's script. The highlighter is a chunk of its own, which the
// page loads in the background once the first conversation has shown
// (`infra/region-load.ts`), and code shown starts loading it if it is not there
// yet, and then its language. A block of code is a region (`region-wait.ts`)
// of the language it names, which needs the highlighter and then the language:
// for 300 ms at most, in all, it draws nothing yet, its text laid out as it
// will be; past that, plain text in the theme's text color, which stays until
// the colors replace it. A block's frame and its language label show at once,
// the label being the language's full name from the first render. Text a
// person types or edits is no region and is never held (`typedCodeRuns`).

let highlighter: Highlighter | null = null
let loading: Promise<void> | null = null

/**
 * Loads the highlighter once; code renders again when it is there. One that
 * fails to load leaves code plain, as a language that fails to load does.
 */
export function loadHighlighter(): Promise<void> {
  loading ??= import('./highlighter').then(
    ({ createHighlighter }) => {
      highlighter = createHighlighter(renderAgain)
      regionsChanged()
      renderAgain()
    },
    reportSyntaxColorsUnavailable,
  )
  return loading
}

/**
 * Loads the highlighter and its common languages, each once, which the page
 * does in the background after the first conversation has shown.
 */
export async function loadHighlighterWithCommonLanguages(): Promise<void> {
  await loadHighlighter()
  await highlighter?.loadCommonLanguages()
}

/**
 * The highlighter, null until it has loaded: the first call starts its load.
 */
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
 * How the code of a fence draws now: `final` once the highlighter and the
 * language are in, or when the fence names no language; a fence of no
 * language at all has nothing to wait for.
 */
export function codeForm(fence: string): RegionForm {
  if (fence === '')
    return 'final'
  void loadedHighlighter()
  return regionForm(`code:${fence.toLowerCase()}`, () => highlighter?.languageReady(fence) ?? false, renderAgain)
}

/** The color of code's text: none while its region is held, the theme's text color otherwise. */
export function codeColor(fence: string): string {
  return codeForm(fence) === 'held' ? 'transparent' : codeTextColor()
}

/**
 * Code as HTML, in the editor's classes and its text color: the colors once
 * its region is final, plain text in that color before, hidden while the
 * region is held, and plain text for a language the highlighter does not know.
 */
export function codeToHtml(code: string, lang: string): string {
  const colored = loadedHighlighter()?.html(code, lang)
  const html = codeForm(lang) === 'final' && colored !== undefined ? colored : escapeHtml(code)
  return `<pre style="color: ${codeColor(lang)}" tabindex="0"><code>${html}</code></pre>`
}

/**
 * A fenced block as a message or a document shows it: highlighted, under the
 * full name of the language its fence names, when it names one. The frame and
 * the label show at once, the label from the table of names in the main bundle
 * (`code-labels.ts`), the name it keeps. The frame is where
 * the page puts its Copy (`CodeBlockCopy.vue`), level with the language.
 */
export function codeBlockHtml(code: string, lang: string): string {
  const fence = lang.trim().split(/\s/, 1)[0] ?? ''
  // The code first: its render starts the highlighter's load.
  const pre = codeToHtml(code, fence)
  const label = fence ? `<div class="code-block-lang">${escapeHtml(fenceLabel(fence))}</div>` : ''
  return `<div class="code-block">${label}${pre}</div>`
}

/**
 * A highlighted run of code: where it starts and ends in the code, and the
 * classes that color it.
 */
export interface CodeRun {
  from: number
  to: number
  classes: string
}

/**
 * Code's highlighted runs in the page's mode, as a code block colors them;
 * the text between them is in `codeColor`. None until its region is final, and
 * for a language the highlighter does not know.
 */
export function codeRuns(code: string, lang: string): CodeRun[] {
  return codeForm(lang) === 'final' ? typedCodeRuns(code, lang) : []
}

/**
 * The runs of text a person types or edits, as soon as the highlighter and the
 * language are in. Such text is no region: it shows in its text color from the
 * first moment and is never held, and the colors join it as they arrive.
 */
export function typedCodeRuns(code: string, lang: string): CodeRun[] {
  return loadedHighlighter()?.runs(code, lang) ?? []
}
