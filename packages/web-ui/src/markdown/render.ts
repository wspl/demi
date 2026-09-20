import { Marked, type RendererObject } from 'marked'
import markedKatex from 'marked-katex-extension'
import type { MarkdownRenderOptions } from './types'
import { codeToHtml } from './highlight'
import { isHttpUrl, messageHostPath, messageImage } from './filePath'
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
/** Links open around the renderer: an image inside one follows it instead of opening itself. */
let openLinks = 0

/** A link that leaves for the web, in a new tab. */
function webLink(url: string): string {
  return `<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer">`
}

/** A link that opens a Host file in the File view. */
function fileLink(path: string): string {
  return `<a href="${escapeHtml(path)}" data-file-link>`
}

/** The Host path a target names, when the message has a Host to resolve it on. */
function hostPath(target: string): string | null {
  const files = activeOptions?.files
  return files ? messageHostPath(target, files.cwd) : null
}

/**
 * Where an image loads from, and the link a click on it follows to show it
 * whole. Null shows its alt text.
 */
function imageSource(target: string): { src: string; link: string | null } | null {
  const image = messageImage(target, activeOptions?.files)
  if (!image)
    return null
  if (!image.opens)
    return { src: image.src, link: null }
  return { src: image.src, link: 'web' in image.opens ? webLink(image.opens.web) : fileLink(image.opens.file) }
}

/**
 * An agent's message: read-only task boxes, HTML shown as text, highlighted
 * code, and links and images that reach the web or the conversation's Host
 * (`file-previews.md` § Files named in messages).
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
    openLinks += 1
    let body: string
    try {
      body = this.parser.parseInline(token.tokens)
    } finally {
      openLinks -= 1
    }
    if (isHttpUrl(token.href))
      return `${webLink(token.href)}${body}</a>`
    const path = hostPath(token.href)
    return path === null ? body : `${fileLink(path)}${body}</a>`
  },
  image(token) {
    const source = imageSource(token.href)
    if (source === null)
      return escapeHtml(token.text)
    const title = token.title ? ` title="${escapeHtml(token.title)}"` : ''
    const image = `<img src="${escapeHtml(source.src)}" alt="${escapeHtml(token.text)}"${title} />`
    return openLinks > 0 || source.link === null ? image : `${source.link}${image}</a>`
  },
}

const agentMarked = new Marked({ gfm: true, breaks: true, renderer: messageRenderer })
agentMarked.use(katexExtension)

/** An agent's message: GitHub Flavored Markdown with math. */
export function renderMarkdown(src: string, options?: MarkdownRenderOptions): string {
  activeOptions = options
  try {
    return agentMarked.parse(src, { async: false })
  } finally {
    activeOptions = undefined
  }
}
