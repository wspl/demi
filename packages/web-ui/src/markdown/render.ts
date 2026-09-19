import { Marked, type RendererObject } from 'marked'
import markedKatex from 'marked-katex-extension'
import type { MarkdownRenderOptions } from './types'
import { codeToHtml } from './highlight'
import { isHttpUrl, messageHostPath } from './filePath'
import { escapeHtml } from './html'

// `$...$` inline / `$$...$$` block LaTeX, rendered to self-contained HTML (KaTeX CSS is loaded
// by web-ui's base stylesheet). `nonStandard` lets inline math sit flush against CJK text the
// model writes; `throwOnError` keeps malformed math from blowing up the whole message.
const katexExtension = markedKatex({
  throwOnError: false,
  nonStandard: true,
  output: 'html'
})

// Parsing is synchronous, so the renderers read the options of the parse in flight instead of
// building a Marked instance (and re-registering KaTeX) per call.
let activeOptions: MarkdownRenderOptions | undefined

/** The Host path a link names, when the message has a Host to resolve it on. */
function linkPath(target: string): string | null {
  const files = activeOptions?.files
  return files ? messageHostPath(target, files.cwd) : null
}

/** Where an image loads from: the web, a `data:` URL as it is, or the Host; null shows its alt text. */
function imageSource(target: string): string | null {
  if (isHttpUrl(target) || target.startsWith('data:'))
    return target
  const files = activeOptions?.files
  if (!files)
    return null
  const path = messageHostPath(target, files.cwd)
  return path === null ? null : files.imageUrl(path)
}

/**
 * What the agent's and the user's messages share: read-only task boxes, HTML
 * shown as text, highlighted code, and links and images that reach the web or
 * the conversation's Host (`file-previews.md` § Files named in messages).
 */
const messageRenderer: RendererObject = {
  checkbox({ checked }) {
    return `<input type="checkbox" disabled${checked ? ' checked' : ''}> `
  },
  html({ text }) {
    return escapeHtml(text)
  },
  code({ text, lang }) {
    return codeToHtml(text, lang ?? '')
  },
  link(token) {
    const body = this.parser.parseInline(token.tokens)
    if (isHttpUrl(token.href))
      return `<a href="${escapeHtml(token.href)}" target="_blank" rel="noopener noreferrer">${body}</a>`
    const path = linkPath(token.href)
    return path === null ? body : `<a href="${escapeHtml(path)}" data-file-link>${body}</a>`
  },
  image(token) {
    const alt = escapeHtml(token.text)
    const src = imageSource(token.href)
    if (src === null)
      return alt
    const title = token.title ? ` title="${escapeHtml(token.title)}"` : ''
    return `<img src="${escapeHtml(src)}" alt="${alt}"${title} />`
  },
}

const agentMarked = new Marked({ gfm: true, breaks: true, renderer: messageRenderer })
agentMarked.use(katexExtension)

// Deliberately no KaTeX on user content: people type `$` for shell vars ($PATH), prices,
// and when discussing LaTeX itself, so rendering math here causes far more false positives
// than it's worth.
const userMarked = new Marked({ gfm: true, breaks: true, renderer: messageRenderer })

/** A user types lists, headings and quotes as plain lines; only inline Markdown applies. */
function escapeBlockSyntax(src: string): string {
  return src
    .replace(/^(\d+)([.)]) /gm, '$1\\$2 ')
    .replace(/^([-*+]) /gm, '\\$1 ')
    .replace(/^(#{1,6}) /gm, '\\$1 ')
    .replace(/^(>)/gm, '\\$1')
}

function parseWith(marked: Marked, src: string, options: MarkdownRenderOptions | undefined): string {
  activeOptions = options
  try {
    return marked.parse(src, { async: false })
  } finally {
    activeOptions = undefined
  }
}

/** An agent's message: GitHub Flavored Markdown with math. */
export function renderMarkdown(src: string, options?: MarkdownRenderOptions): string {
  return parseWith(agentMarked, src, options)
}

/** A user's message: the same, without math or block syntax. */
export function renderUserMarkdown(src: string, options?: MarkdownRenderOptions): string {
  return parseWith(userMarked, escapeBlockSyntax(src), options)
}
