import DOMPurify from 'dompurify'
import { renderMarkdown } from './render'
import type { MarkdownRenderOptions } from './types'

// A web link opens in a new tab: the renderer gives it `target="_blank"` with
// `rel="noopener noreferrer"`, and `target` is the one attribute that needs
// adding to DOMPurify's defaults. KaTeX's inline styles and `data-file-link`
// pass them as they are.
const SANITIZE = { ADD_ATTR: ['target'] }

/** An agent's messages as the page shows them: rendered, then sanitized. */
export const md = {
  render(src: string, options?: MarkdownRenderOptions): string {
    return DOMPurify.sanitize(renderMarkdown(src, options), SANITIZE)
  },
}
