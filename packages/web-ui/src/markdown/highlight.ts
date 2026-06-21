import { readonly, shallowRef } from 'vue'
import { createHighlighter, type BundledLanguage, type BundledTheme, type Highlighter } from 'shiki'
import { codeThemes, findCodeTheme } from '../theme/codeThemes'
import type { MarkdownThemeSnapshot } from './types'

const ALL_SHIKI_THEMES: BundledTheme[] = [
  ...new Set(codeThemes.flatMap((theme) => [theme.dark.shikiTheme, theme.light.shikiTheme])),
]

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

const highlighterReady = createHighlighter({ themes: ALL_SHIKI_THEMES, langs: [...LANGS] }).then((instance) => {
  highlighter.value = instance
  renderVersion.value += 1
})

function escapeHtml(str: string): string {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

function detectThemeMode(): 'light' | 'dark' {
  if (typeof document === 'undefined') return 'dark'
  return document.documentElement.getAttribute('data-theme') === 'light' ? 'light' : 'dark'
}

function getShikiTheme(theme?: MarkdownThemeSnapshot): BundledTheme {
  const snapshot = theme ?? { mode: detectThemeMode(), codeThemeId: codeThemes[0]?.id ?? 'one' }
  const definition = findCodeTheme(snapshot.codeThemeId)
  return snapshot.mode === 'light' ? definition.light.shikiTheme : definition.dark.shikiTheme
}

function resolveLang(lang: string): string {
  if (!lang || lang === 'text' || lang === 'plaintext') return 'text'
  const instance = highlighter.value
  if (!instance) return 'text'
  const resolved = LANG_ALIASES[lang] ?? lang
  return instance.getLoadedLanguages().includes(resolved as BundledLanguage) ? resolved : 'text'
}

export function codeToHtml(code: string, lang: string, theme?: MarkdownThemeSnapshot): string {
  const instance = highlighter.value
  if (!instance) return `<pre><code>${escapeHtml(code)}</code></pre>`

  return instance.codeToHtml(code, {
    lang: resolveLang(lang),
    theme: getShikiTheme(theme),
    transformers: [{
      pre(node) {
        const style = String(node.properties?.['style'] ?? '')
        node.properties['style'] = style.replace(/background-color:[^;]+;?\s*/g, '').trim() || undefined
      },
    }],
  })
}

export interface TokenSpan {
  content: string
  color?: string
}

export function codeToTokenLines(code: string, lang: string, theme?: MarkdownThemeSnapshot): TokenSpan[][] {
  const instance = highlighter.value
  if (!instance) {
    return splitLines(code).map((line) => [{ content: line }])
  }
  const resolved = resolveLang(lang) as BundledLanguage
  const { tokens } = instance.codeToTokens(code, { lang: resolved, theme: getShikiTheme(theme) })
  return tokens.map((line) => line.map((t): TokenSpan => t.color ? { content: t.content, color: t.color } : { content: t.content }))
}

const EXT_TO_LANG: Record<string, string> = {
  '.ts': 'typescript', '.tsx': 'tsx', '.js': 'javascript', '.jsx': 'jsx',
  '.cjs': 'javascript', '.mjs': 'javascript',
  '.json': 'json', '.jsonc': 'jsonc',
  '.vue': 'vue', '.html': 'html', '.htm': 'html',
  '.css': 'css', '.scss': 'scss', '.less': 'less',
  '.md': 'markdown',
  '.py': 'python', '.rb': 'ruby', '.rs': 'rust', '.go': 'go',
  '.java': 'java', '.kt': 'kotlin', '.swift': 'swift', '.dart': 'dart', '.php': 'php',
  '.c': 'c', '.h': 'c', '.cpp': 'cpp', '.cc': 'cpp', '.hpp': 'cpp', '.cs': 'csharp',
  '.sh': 'bash', '.zsh': 'bash', '.bash': 'bash',
  '.yml': 'yaml', '.yaml': 'yaml',
  '.xml': 'xml', '.svg': 'xml',
  '.sql': 'sql', '.graphql': 'graphql',
  '.toml': 'toml', '.ini': 'ini', '.env': 'ini',
}

const FILENAME_TO_LANG: Record<string, string> = {
  Dockerfile: 'dockerfile',
  Makefile: 'makefile',
  '.gitignore': 'ini',
}

export function getLanguageFromPath(filepath: string): string {
  const filename = filepath.split('/').pop() ?? ''
  const byName = FILENAME_TO_LANG[filename]
  if (byName) return byName
  const dotIndex = filename.lastIndexOf('.')
  if (dotIndex <= 0) return 'text'
  return EXT_TO_LANG[filename.slice(dotIndex).toLowerCase()] ?? 'text'
}

export function useMarkdownRenderVersion() {
  return readonly(renderVersion)
}

export async function waitForMarkdownHighlighter() {
  await highlighterReady
}

function splitLines(code: string): string[] {
  const lines = code.split('\n')
  if (lines.at(-1) === '') lines.pop()
  return lines
}
