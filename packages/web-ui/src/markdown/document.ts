// A Markdown file rendered the way GitHub shows a repository file
// (`file-previews.md` § Markdown). Unlike a message, the file's own HTML
// renders, so the output passes an allowlist; math and code render after it,
// from their text, because their output depends on inline styles the
// allowlist removes.
import DOMPurify, { type DOMPurify as Purifier } from 'dompurify'
import katex from 'katex'
import { Marked, type MarkedExtension, type Tokens } from 'marked'
import { gfmHeadingId } from 'marked-gfm-heading-id'
import markedKatex from 'marked-katex-extension'
import { utf8Bytes } from '@demicodes/utils'
import { joinPath, parentPath } from '../files/paths'
import { codeToHtml } from './highlight'
import { escapeHtml } from './html'

/** Where a document sits, and how an image it names on the Host loads. */
export interface DocumentPlace {
  /** The document's own path; a relative target resolves against its directory. */
  path: string
  /**
   * The workspace root: a target starting with `/` resolves against it, as
   * GitHub resolves one against the repository root.
   */
  root: string
  /** The URL an image at a Host path loads from. */
  imageUrl(path: string): string
}

/** Past this size a document shows only its source: rendering that much stalls the page. */
export const DOCUMENT_RENDER_BYTES = 2 * 1024 * 1024

export function renderable(text: string): boolean {
  return utf8Bytes(text) <= DOCUMENT_RENDER_BYTES
}

/**
 * The prefix `SANITIZE_NAMED_PROPS` gives the document's ids and anchor
 * names, so they never collide with the page's; `#` links follow it.
 */
const ID_PREFIX = 'user-content-'

/**
 * The elements and attributes that pass, modeled on GitHub's allowlist: no
 * script, event handler, `style`, form, frame or embedded object.
 */
const ALLOWED_TAGS = [
  'a', 'abbr', 'b', 'blockquote', 'br', 'caption', 'cite', 'code', 'dd', 'del',
  'details', 'dfn', 'div', 'dl', 'dt', 'em', 'figcaption', 'figure', 'h1', 'h2',
  'h3', 'h4', 'h5', 'h6', 'hr', 'i', 'img', 'input', 'ins', 'kbd', 'li', 'mark',
  'ol', 'p', 'picture', 'pre', 'q', 'rp', 'rt', 'ruby', 's', 'samp', 'small',
  'source', 'span', 'strike', 'strong', 'sub', 'summary', 'sup', 'table',
  'tbody', 'td', 'tfoot', 'th', 'thead', 'time', 'tr', 'tt', 'u', 'ul', 'var',
  'wbr',
]
const ALLOWED_ATTR = [
  'align', 'alt', 'checked', 'cite', 'class', 'colspan', 'datetime', 'dir',
  'disabled', 'height', 'href', 'id', 'lang', 'media', 'name', 'open',
  'reversed', 'rowspan', 'scope', 'src', 'srcset', 'start', 'title', 'type',
  'valign', 'width',
]
/** The classes the renderer gives; any other class a document names is dropped. */
const RENDERER_CLASS = /^(math|math-inline|math-display|language-[\w+#.-]+)$/

// Math placeholders keep the extension's syntax, standard `$` spacing
// included, and hold the TeX as text until sanitizing is done.
const mathPlaceholders: MarkedExtension = {
  extensions: (markedKatex({ throwOnError: false }).extensions ?? []).map((extension) => ({
    ...extension,
    renderer: (token: Tokens.Generic) => {
      const tex = escapeHtml(String(token['text']))
      return token['displayMode']
        ? `<div class="math math-display">${tex}</div>\n`
        : `<span class="math math-inline">${tex}</span>`
    },
  })),
}

const documentMarked = new Marked({ gfm: true, breaks: false }, gfmHeadingId(), mathPlaceholders)

// Hooks belong to a DOMPurify instance, and sanitizing is synchronous, so the
// hooks read the place of the render in progress.
let purifier: Purifier | null = null
let active: DocumentPlace | null = null

/** The document as HTML the page can show as it is. */
export function renderMarkdownDocument(text: string, place: DocumentPlace): string {
  const html = documentMarked.parse(frontMatterAsCode(text), { async: false })
  active = place
  let fragment: DocumentFragment
  try {
    fragment = documentPurifier().sanitize(html, {
      ALLOWED_TAGS,
      ALLOWED_ATTR,
      SANITIZE_NAMED_PROPS: true,
      RETURN_DOM_FRAGMENT: true,
    })
  } finally {
    active = null
  }
  renderAfterSanitizing(fragment)
  const container = document.createElement('div')
  container.append(fragment)
  return container.innerHTML
}

function documentPurifier(): Purifier {
  if (purifier)
    return purifier
  const instance = DOMPurify(window)
  instance.addHook('uponSanitizeAttribute', (_node, data) => {
    const place = active
    if (!place)
      return
    switch (data.attrName) {
      case 'href': {
        const href = linkTarget(data.attrValue, place)
        if (href === null)
          data.keepAttr = false
        else
          data.attrValue = href
        break
      }
      case 'src': {
        const src = imageTarget(data.attrValue, place)
        if (src === null)
          data.keepAttr = false
        else
          data.attrValue = src
        break
      }
      case 'srcset': {
        const candidates = data.attrValue.split(',').flatMap((candidate) => {
          const [url, ...descriptor] = candidate.trim().split(/\s+/)
          const src = url ? imageTarget(url, place) : null
          return src === null ? [] : [[src, ...descriptor].join(' ')]
        })
        if (candidates.length === 0)
          data.keepAttr = false
        else
          data.attrValue = candidates.join(', ')
        break
      }
      case 'class': {
        const kept = data.attrValue.split(/\s+/).filter((name) => RENDERER_CLASS.test(name))
        if (kept.length === 0)
          data.keepAttr = false
        else
          data.attrValue = kept.join(' ')
        break
      }
    }
  })
  instance.addHook('afterSanitizeAttributes', (node) => {
    if (node.nodeName === 'A') {
      const href = node.getAttribute('href') ?? ''
      if (isExternal(href)) {
        node.setAttribute('target', '_blank')
        node.setAttribute('rel', 'noopener noreferrer')
      } else if (href.startsWith('/')) {
        node.setAttribute('data-file-link', '')
      }
    }
    // A task list's checkbox is the one input that passes, and it cannot be ticked.
    if (node.nodeName === 'INPUT') {
      if (node.getAttribute('type') === 'checkbox')
        node.setAttribute('disabled', '')
      else
        node.remove()
    }
  })
  purifier = instance
  return instance
}

/** Math and code, rendered from their text: nothing a document wrote reaches their output. */
function renderAfterSanitizing(fragment: DocumentFragment): void {
  for (const element of fragment.querySelectorAll<HTMLElement>('.math')) {
    katex.render(element.textContent ?? '', element, {
      displayMode: element.classList.contains('math-display'),
      throwOnError: false,
    })
  }
  for (const code of fragment.querySelectorAll<HTMLElement>('pre > code')) {
    const language = [...code.classList].find((name) => name.startsWith('language-'))?.slice('language-'.length) ?? ''
    const pre = code.parentElement!
    const text = code.textContent ?? ''
    if (language === 'math') {
      // A `math` block is display math, as on GitHub.
      const block = document.createElement('div')
      katex.render(text, block, { displayMode: true, throwOnError: false })
      pre.replaceWith(block)
      continue
    }
    const highlighted = document.createElement('template')
    highlighted.innerHTML = codeToHtml(text, language)
    pre.replaceWith(highlighted.content)
  }
}

function isExternal(target: string): boolean {
  return /^https?:\/\//i.test(target) || target.startsWith('//')
}

/** A scheme other than a web one, such as `javascript:` or `data:`. */
function hasScheme(target: string): boolean {
  return /^[a-z][a-z0-9+.-]*:/i.test(target)
}

/**
 * What a link leads to (`file-previews.md` § Markdown): a heading of this
 * document, a web page, or a Host path; null for anything else, whose text
 * stays without the link.
 */
export function linkTarget(target: string, place: DocumentPlace): string | null {
  const trimmed = target.trim()
  if (trimmed.startsWith('#'))
    return trimmed === '#' ? null : `#${ID_PREFIX}${decoded(trimmed.slice(1))}`
  if (isExternal(trimmed))
    return trimmed
  if (hasScheme(trimmed))
    return null
  return hostPath(trimmed, place)
}

/** Where an image loads from: the web, or the Host through `imageUrl`; null drops it. */
export function imageTarget(target: string, place: DocumentPlace): string | null {
  const trimmed = target.trim()
  if (isExternal(trimmed))
    return trimmed
  if (hasScheme(trimmed) || trimmed.startsWith('#'))
    return null
  const path = hostPath(trimmed, place)
  return path === null ? null : place.imageUrl(path)
}

/** The Host path a relative or `/`-rooted target names, its query and fragment dropped. */
function hostPath(target: string, place: DocumentPlace): string | null {
  const file = decoded(target.split(/[?#]/)[0] ?? '')
  if (file === '')
    return null
  return file.startsWith('/') ? joinPath(place.root, file) : joinPath(parentPath(place.path), file)
}

/** A target as written may be percent-encoded; one that does not decode stays as it is. */
function decoded(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

/** Leading YAML front matter as a YAML code block, fenced past any backticks inside it. */
export function frontMatterAsCode(text: string): string {
  const match = text.match(/^---\r?\n([\s\S]*?)\r?\n(?:---|\.\.\.)[ \t]*(?:\r?\n|$)/)
  if (!match)
    return text
  const yaml = match[1] ?? ''
  const longest = Math.max(2, ...[...yaml.matchAll(/`+/g)].map((run) => run[0].length))
  const fence = '`'.repeat(longest + 1)
  return `${fence}yaml\n${yaml}\n${fence}\n${text.slice(match[0].length)}`
}
