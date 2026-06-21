import { Marked } from 'marked'
import DOMPurify from 'dompurify'
import { renderMarkdown } from './render'
import { isHttpUrl } from './filePath'
import { codeToHtml } from './highlight'
import type { MarkdownRenderOptions } from './types'

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

const userMarked = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    html({ text }) {
      return escapeHtml(text)
    },
    code({ text, lang }) {
      return codeToHtml(text, lang ?? '')
    },
    codespan({ text }) {
      return `<code>${escapeHtml(text)}</code>`
    },
    link(token) {
      const href = token.href
      const body = this.parser.parseInline(token.tokens)
      if (isHttpUrl(href)) {
        return `<a href="${escapeHtml(href)}" target="_blank" rel="noopener noreferrer">${body}</a>`
      }
      return body
    },
  },
})

function escapeBlockSyntax(src: string): string {
  return src
    .replace(/^(\d+)([.)]) /gm, '$1\\$2 ')
    .replace(/^([-*+]) /gm, '\\$1 ')
    .replace(/^(#{1,6}) /gm, '\\$1 ')
    .replace(/^(>)/gm, '\\$1')
}

export const md = {
  render(src: string, options?: MarkdownRenderOptions): string {
    return DOMPurify.sanitize(renderMarkdown(src, options), { ADD_ATTR: ['style', 'data-file-link'] })
  },
  renderUser(src: string): string {
    return DOMPurify.sanitize(userMarked.parse(escapeBlockSyntax(src), { async: false }) as string)
  },
}
