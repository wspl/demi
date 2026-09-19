import { readonly, shallowRef } from 'vue'
import { createHighlighter, type Highlighter } from 'shiki'
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
  (error: unknown) => reportError('Code highlighting is unavailable', error),
)

// Code shows in the app's mode, so a mode switch is a new render too. The
// subscription lasts as long as the page, like the highlighter.
appThemeStore.subscribe(() => {
  renderVersion.value += 1
})

function resolveLang(lang: string): string {
  if (!lang || lang === 'text' || lang === 'plaintext')
    return 'text'
  const instance = highlighter.value
  if (!instance)
    return 'text'
  const resolved = LANG_ALIASES[lang] ?? lang
  return instance.getLoadedLanguages().includes(resolved)
    ? resolved
    : 'text'
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

/** Changes when rendered code should render again: the highlighter's arrival and the page's mode. */
export function useMarkdownRenderVersion() {
  return readonly(renderVersion)
}
