import { Marked } from 'marked'
import markedKatex from 'marked-katex-extension'
import type { MarkdownRenderOptions } from './types'
import { codeToHtml } from './highlight'
import { isHttpUrl, isLikelyFilePath, normalizeFilePath, resolveAbsolutePath, toLocalFileUrl } from './filePath'

// `$...$` inline / `$$...$$` block LaTeX, rendered to self-contained HTML (KaTeX CSS is loaded
// by the app). `nonStandard` lets inline math sit flush against CJK text the model writes;
// `throwOnError` keeps malformed math from blowing up the whole message.
const katexExtension = markedKatex({ throwOnError: false, nonStandard: true, output: 'html' })

const INLINE_CODE_RE = /`([^`\n]+)`/g
const MARKDOWN_LINK_RE = /\[[^\]]*]\(([^)]+)\)/g

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

function extractLinkHref(rawTarget: string): string {
  const trimmed = rawTarget.trim()
  if (!trimmed) return ''

  const unwrapped = trimmed.startsWith('<') && trimmed.endsWith('>')
    ? trimmed.slice(1, -1)
    : trimmed
  const separatorIndex = unwrapped.search(/\s/)
  return separatorIndex === -1 ? unwrapped : unwrapped.slice(0, separatorIndex)
}

export function extractFilePathCandidates(src: string): Set<string> {
  const candidates = new Set<string>()

  for (const match of src.matchAll(INLINE_CODE_RE)) {
    const codeText = match[1]
    if (!codeText || !isLikelyFilePath(codeText)) continue
    const normalizedPath = normalizeFilePath(codeText)
    if (normalizedPath) candidates.add(normalizedPath)
  }

  for (const match of src.matchAll(MARKDOWN_LINK_RE)) {
    const rawHref = match[1]
    if (!rawHref) continue
    const href = extractLinkHref(rawHref)
    if (!isLikelyFilePath(href)) continue
    const normalizedPath = normalizeFilePath(href)
    if (normalizedPath) candidates.add(normalizedPath)
  }

  return candidates
}

function resolveImageSource(href: string, basePath?: string): string {
  const trimmedHref = href.trim()
  if (!trimmedHref) return ''
  if (isHttpUrl(trimmedHref) || trimmedHref.startsWith('data:') || trimmedHref.startsWith('local-file:')) return trimmedHref
  if (!basePath) return trimmedHref
  const absPath = resolveAbsolutePath(basePath, trimmedHref)
  return absPath ? toLocalFileUrl(absPath) : trimmedHref
}

function createMarked(options?: MarkdownRenderOptions) {
  const knownPaths = options?.knownPaths
  const basePath = options?.basePath

  const marked = new Marked({
    gfm: true,
    breaks: true,
    renderer: {
      html({ text }) {
        return escapeHtml(text)
      },
      code({ text, lang }) {
        return codeToHtml(text, lang ?? '', options?.theme)
      },
      codespan({ text }) {
        const normalizedPath = normalizeFilePath(text)
        if (isLikelyFilePath(text) && knownPaths?.has(normalizedPath)) {
          return `<a class="file-link" href="${escapeHtml(normalizedPath)}" data-file-link>${escapeHtml(text)}</a>`
        }
        return `<code>${escapeHtml(text)}</code>`
      },
      link(token) {
        const href = extractLinkHref(token.href)
        const body = this.parser.parseInline(token.tokens)
        if (isHttpUrl(href)) {
          return `<a href="${escapeHtml(href)}" target="_blank" rel="noopener noreferrer">${body}</a>`
        }
        if (isLikelyFilePath(href)) {
          const normalizedPath = normalizeFilePath(href)
          if (!knownPaths || knownPaths.has(normalizedPath)) {
            return `<a href="${escapeHtml(normalizedPath)}" data-file-link>${body}</a>`
          }
        }
        return body
      },
      image(token) {
        const href = extractLinkHref(token.href)
        const safeSrc = resolveImageSource(href, basePath)
        if (!safeSrc) return escapeHtml(token.text)

        const title = token.title ? ` title="${escapeHtml(token.title)}"` : ''
        const alt = escapeHtml(token.text)
        return `<img src="${escapeHtml(safeSrc)}" alt="${alt}"${title} />`
      },
    },
  })
  marked.use(katexExtension)
  return marked
}

export function renderMarkdown(src: string, options?: MarkdownRenderOptions): string {
  return createMarked(options).parse(src, { async: false }) as string
}
