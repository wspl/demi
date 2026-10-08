import { readonly, shallowRef } from 'vue'
import { bundledLanguagesInfo, createHighlighter, getTokenStyleObject, stringifyTokenStyle, type Highlighter } from 'shiki'
import { reportError } from '../infra/errors'
import { appThemeStore } from '../theme/appTheme'
import { codeTheme } from '../theme/codeTheme'
import { escapeHtml } from './html'

const LANGS = [
  'typescript', 'javascript', 'tsx', 'jsx', 'json', 'jsonc',
  'html', 'css', 'scss', 'less', 'vue',
  'python', 'ruby', 'rust', 'go', 'java', 'kotlin', 'swift', 'dart', 'php',
  'c', 'cpp', 'csharp',
  'bash',
  'yaml', 'toml', 'ini', 'xml', 'sql', 'graphql',
  'markdown', 'dockerfile', 'makefile',
] as const

const LANG_ALIASES: Record<string, string> = {
  shell: 'bash',
  sh: 'bash',
  zsh: 'bash',
  js: 'javascript',
  ts: 'typescript',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  cs: 'csharp',
  yml: 'yaml',
}

const highlighter = shallowRef<Highlighter | null>(null)
const renderVersion = shallowRef(0)

// Until the highlighter arrives, and if it never does, code shows as plain text.
createHighlighter({
  themes: [codeTheme.dark.shikiTheme, codeTheme.light.shikiTheme],
  langs: [...LANGS],
}).then(
  (instance) => {
    highlighter.value = instance
    renderVersion.value += 1
  },
  (error: unknown) => reportError('Code Highlighting Is Unavailable', error),
)

// Code shows in the app's mode, so a mode switch is a new render too. The
// subscription lasts as long as the page, like the highlighter.
appThemeStore.subscribe(() => {
  renderVersion.value += 1
})

/** The loaded grammar a fence's language names, directly or by a short name; plain text for any other. */
function resolveLang(lang: string): (typeof LANGS)[number] | 'text' {
  const resolved = LANG_ALIASES[lang] ?? lang
  return LANGS.find((each) => each === resolved) ?? 'text'
}

export function codeToHtml(code: string, lang: string): string {
  const instance = highlighter.value
  if (!instance)
    return `<pre><code>${escapeHtml(code)}</code></pre>`

  return instance.codeToHtml(code, {
    lang: resolveLang(lang),
    theme: codeTheme[appThemeStore.state.mode].shikiTheme,
    transformers: [{
      pre(node) {
        const style = String(node.properties?.['style'] ?? '')
        node.properties['style'] = style.replace(/background-color:[^;]+;?\s*/g, '').trim() || undefined
      },
    }],
  })
}

/**
 * A language's full name, as Shiki knows it by its id or any of its short
 * names (`ts` is TypeScript); one Shiki does not know is shown as the fence
 * wrote it.
 */
function languageName(lang: string): string {
  const key = lang.toLowerCase()
  const info = bundledLanguagesInfo.find((each) => each.id === key || each.aliases?.includes(key))
  return info?.name ?? lang
}

/**
 * A fenced block as a message or a document shows it: highlighted, under the
 * full name of the language its fence names, when it names one. The frame is
 * where the page puts its Copy (`CodeBlockCopy.vue`), level with the language.
 */
export function codeBlockHtml(code: string, lang: string): string {
  const fence = lang.trim().split(/\s/, 1)[0] ?? ''
  const label = fence ? `<div class="code-block-lang">${escapeHtml(languageName(fence))}</div>` : ''
  return `<div class="code-block">${label}${codeToHtml(code, fence)}</div>`
}

/**
 * Code's highlighted runs in the app's mode: where each starts and ends in
 * the code, and the CSS that colors it. Null until the highlighter arrives.
 */
export function codeStyles(code: string, lang: string): { from: number; to: number; style: string }[] | null {
  const instance = highlighter.value
  if (!instance)
    return null
  const { tokens } = instance.codeToTokens(code, {
    lang: resolveLang(lang),
    theme: codeTheme[appThemeStore.state.mode].shikiTheme,
  })
  return tokens.flat().flatMap((token) => {
    const style = stringifyTokenStyle(getTokenStyleObject(token))
    return style ? [{ from: token.offset, to: token.offset + token.content.length, style }] : []
  })
}

/** Changes when rendered code should render again: the highlighter's arrival and the page's mode. */
export function useMarkdownRenderVersion() {
  return readonly(renderVersion)
}
