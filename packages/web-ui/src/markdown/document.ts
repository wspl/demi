// A Markdown file rendered the way GitHub shows a repository file
// (`file-previews.md` § Markdown). Unlike a message, the file's own HTML
// renders, so the output passes an allowlist; math and code render after it,
// from their text, because their output depends on inline styles the
// allowlist removes.
import DOMPurify, { type DOMPurify as Purifier } from 'dompurify'
import katex from 'katex'
import type { MarkedExtension, Token, Tokens } from 'marked'
import { gfmHeadingId } from 'marked-gfm-heading-id'
import { z } from 'zod'
import { utf8Bytes } from '@demicodes/utils'
import { t } from '../files/locales/text'
import { joinPath, parentPath } from '../files/paths'
import type { ThemeMode } from '../theme/appTheme'
import { decodedTarget } from './filePath'
import { codeBlockHtml } from './highlight'
import { escapeHtml } from './html'
import { mathExtensions } from './math'
import { markdownReader } from './reader'
import { scrollingTable } from './table'

/** Where a document sits, against which the Host paths it names resolve. */
export interface DocumentPlace {
  /** The document's own path; a relative target resolves against its directory. */
  path: string
  /**
   * The workspace root: a target starting with `/` resolves against it, as
   * GitHub resolves one against the repository root.
   */
  root: string
}

/**
 * The attribute a Host image carries in the rendering, holding the path of
 * its file: the image has no address yet, because the address names the
 * version of the file the page keeps (`useHostImagesAt`), which is given
 * to the image where it stands (`MarkdownDocument`) and changes with the file.
 */
export const HOST_FILE_ATTRIBUTE = 'data-host-file'

/**
 * The attribute an `<img>` or a `<picture>`'s `<source>` carries in place of
 * a `srcset` that names a Host file, holding its candidates as JSON
 * (`SrcsetCandidate[]`): each Host candidate takes the address of its file's
 * version where it stands, as a Host image's `src` does.
 */
export const HOST_SRCSET_ATTRIBUTE = 'data-host-srcset'

/** One candidate of a `srcset`: an address on the web, or a Host file, and its descriptor such as `2x`. */
const srcsetCandidateSchema = z.union([
  z.object({ url: z.string(), descriptor: z.string() }),
  z.object({ path: z.string(), descriptor: z.string() }),
])
export type SrcsetCandidate = z.infer<typeof srcsetCandidateSchema>
type WebCandidate = Extract<SrcsetCandidate, { url: string }>

/** A document as the page shows it. */
export interface RenderedDocument {
  html: string
  /** The Host paths of the images the document names, `srcset` candidates included, each once. */
  hostFiles: string[]
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
const RENDERER_CLASS = /^(math|math-inline|math-display|table-scroll|markdown-alert(-title|-note|-tip|-important|-warning|-caution)?|language-[\w+#.-]+)$/

/** The kinds of GitHub's alerts, in the words of their marker. */
const ALERT_KINDS = ['note', 'tip', 'important', 'warning', 'caution'] as const
type AlertKind = (typeof ALERT_KINDS)[number]

/** An alert's marker, alone on the first line of a quote. */
const ALERT_MARKER = /^\[!(note|tip|important|warning|caution)\][ \t]*(?:\n|$)/i

/** The quotes that are alerts, by their kind, as the lexer finds them. */
const alertKinds = new WeakMap<Token, AlertKind>()

/**
 * GitHub's alerts: a quote whose first line is `[!NOTE]`, `[!TIP]`,
 * `[!IMPORTANT]`, `[!WARNING]` or `[!CAUTION]` shows as a callout titled
 * with its kind, the marker gone.
 */
const githubAlerts: MarkedExtension = {
  walkTokens(token) {
    // `type` does not narrow marked's tokens, whose union holds an open `Generic`.
    if (token.type !== 'blockquote' || !('tokens' in token) || !token.tokens)
      return
    const paragraph = token.tokens[0]
    if (paragraph?.type !== 'paragraph' || !('tokens' in paragraph) || !paragraph.tokens)
      return
    const first = paragraph.tokens[0]
    if (first?.type !== 'text' || !('text' in first))
      return
    const match = ALERT_MARKER.exec(first.raw)
    const kind = ALERT_KINDS.find((name) => name === match?.[1]?.toLowerCase())
    if (!match || !kind)
      return
    alertKinds.set(token, kind)
    first.raw = first.raw.slice(match[0].length)
    first.text = first.text.replace(ALERT_MARKER, '')
    if (first.raw === '')
      paragraph.tokens.shift()
    if (paragraph.tokens.length === 0)
      token.tokens.shift()
  },
  renderer: {
    blockquote(token) {
      const kind = alertKinds.get(token)
      if (!kind)
        return false
      return `<div class="markdown-alert markdown-alert-${kind}"><p class="markdown-alert-title">${escapeHtml(t(`document.alert.${kind}`))}</p>\n${this.parser.parse(token.tokens)}</div>\n`
    },
  },
}

/** GitHub's fragments that show an image in one theme only. */
const MODE_ONLY = /#gh-(dark|light)-mode-only$/

// Math placeholders keep the messages' math syntax and hold the TeX as text
// until sanitizing is done.
const mathPlaceholders: MarkedExtension = {
  extensions: mathExtensions({ throwOnError: false }).map((extension) => ({
    ...extension,
    renderer: (token: Tokens.Generic) => {
      const tex = escapeHtml(String(token['text']))
      return token['displayMode']
        ? `<div class="math math-display">${tex}</div>\n`
        : `<span class="math math-inline">${tex}</span>`
    },
  })),
}

const documentMarked = markdownReader({ breaks: false }, gfmHeadingId(), mathPlaceholders, scrollingTable, githubAlerts)

/** What a render in progress reads: where the document sits, and the theme the page shows. */
interface ActiveRender {
  place: DocumentPlace
  theme: ThemeMode
}

// Hooks belong to a DOMPurify instance, and sanitizing is synchronous, so the
// hooks read the render in progress.
let purifier: Purifier | null = null
let active: ActiveRender | null = null
/** The Host path of each image of the render in progress, until the hook that sets its attribute. */
const hostFiles = new WeakMap<Node, string>()
/** The `srcset` of each image or source of the render in progress that names a Host file, until the hook that sets its attribute. */
const hostSrcsets = new WeakMap<Node, SrcsetCandidate[]>()
/** The images of the render in progress that show only in the other theme. */
const otherTheme = new WeakSet<Node>()

/**
 * The document as HTML the page can show as it is, and the Host files its
 * images name. `theme` is the theme the page shows, which picks a
 * `<picture>`'s source for a color scheme and GitHub's mode-only images
 * (`file-previews.md` § Files named in messages), so a new theme is a new render.
 */
export function renderMarkdownDocument(text: string, place: DocumentPlace, theme: ThemeMode): RenderedDocument {
  const html = documentMarked.parse(frontMatterAsCode(text), { async: false })
  active = { place, theme }
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
  sourcesForTheme(fragment, theme)
  renderAfterSanitizing(fragment)
  const paths = [
    ...[...fragment.querySelectorAll(`[${HOST_FILE_ATTRIBUTE}]`)].map((image) => image.getAttribute(HOST_FILE_ATTRIBUTE)!),
    ...[...fragment.querySelectorAll(`[${HOST_SRCSET_ATTRIBUTE}]`)].flatMap((element) =>
      hostSrcsetCandidates(element).flatMap((candidate) => 'path' in candidate ? [candidate.path] : [])),
  ]
  const container = document.createElement('div')
  container.append(fragment)
  return { html: container.innerHTML, hostFiles: [...new Set(paths)] }
}

function documentPurifier(): Purifier {
  if (purifier)
    return purifier
  const instance = DOMPurify(window)
  instance.addHook('uponSanitizeAttribute', (node, data) => {
    if (!active)
      return
    const { place, theme } = active
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
        const mode = MODE_ONLY.exec(data.attrValue.trim())?.[1]
        if (mode !== undefined && mode !== theme)
          otherTheme.add(node)
        const image = imageTarget(data.attrValue, place)
        if (image && 'url' in image) {
          data.attrValue = image.url
        } else {
          data.keepAttr = false
          if (image)
            hostFiles.set(node, image.path)
        }
        break
      }
      case 'srcset': {
        const candidates = data.attrValue.split(',').flatMap((candidate): SrcsetCandidate[] => {
          const [target, ...descriptor] = candidate.trim().split(/\s+/)
          const image = target ? imageTarget(target, place) : null
          return image ? [{ ...image, descriptor: descriptor.join(' ') }] : []
        })
        if (candidates.some((candidate) => 'path' in candidate)) {
          // A Host candidate takes its address where it stands, from the version of
          // its file the page keeps, as a Host image's `src` does.
          data.keepAttr = false
          hostSrcsets.set(node, candidates)
        } else if (candidates.length === 0) {
          data.keepAttr = false
        } else {
          data.attrValue = srcsetText(candidates.filter((candidate) => 'url' in candidate))
        }
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
    if (otherTheme.has(node)) {
      node.remove()
      return
    }
    const hostFile = hostFiles.get(node)
    if (hostFile !== undefined && node.nodeName === 'IMG')
      node.setAttribute(HOST_FILE_ATTRIBUTE, hostFile)
    const hostSrcset = hostSrcsets.get(node)
    if (hostSrcset !== undefined && (node.nodeName === 'IMG' || node.nodeName === 'SOURCE'))
      node.setAttribute(HOST_SRCSET_ATTRIBUTE, JSON.stringify(hostSrcset))
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

/** The candidates an element's `HOST_SRCSET_ATTRIBUTE` holds, which the renderer wrote. */
export function hostSrcsetCandidates(element: Element): SrcsetCandidate[] {
  return z.array(srcsetCandidateSchema).parse(JSON.parse(element.getAttribute(HOST_SRCSET_ATTRIBUTE) ?? '[]'))
}

/** A `srcset` of addresses, each with its descriptor. */
export function srcsetText(candidates: WebCandidate[]): string {
  return candidates.map(({ url, descriptor }) => descriptor === '' ? url : `${url} ${descriptor}`).join(', ')
}

/**
 * A `<source>` whose `media` asks for a color scheme, judged by the theme
 * the page shows, not the system's, since the user may show Dark on a light
 * system: a query that asks for the other scheme is false, and one that asks
 * for this one holds as far as the scheme goes. A source none of whose
 * queries can hold goes, and the `<img>` or a later source shows.
 */
function sourcesForTheme(fragment: DocumentFragment, theme: ThemeMode): void {
  for (const source of fragment.querySelectorAll('source[media]')) {
    const media = mediaForTheme(source.getAttribute('media')!, theme)
    if (media === null)
      source.remove()
    else
      source.setAttribute('media', media)
  }
}

const COLOR_SCHEME = /\(\s*prefers-color-scheme\s*:\s*(light|dark)\s*\)/gi

/**
 * A media query list as it reads in `theme`: each query that asks for a
 * color scheme either goes, when it cannot hold in this theme, or keeps its
 * other conditions; null when no query is left. A query that `not` negates
 * is judged on its scheme alone, its other conditions dropped.
 */
export function mediaForTheme(media: string, theme: ThemeMode): string | null {
  if (!/prefers-color-scheme/i.test(media))
    return media
  const queries = media.split(',').flatMap((query) => {
    const schemes = [...query.matchAll(COLOR_SCHEME)].map((match) => match[1]!.toLowerCase())
    if (schemes.length === 0)
      return [query.trim()]
    const asks = schemes.every((scheme) => scheme === theme)
    if (/^\s*not\b/i.test(query))
      return asks ? [] : ['all']
    if (!asks)
      return []
    // The scheme holds; what is left of the query decides, `(min-width: 0)`, always true, standing for the scheme.
    const rest = query.replace(COLOR_SCHEME, '(min-width: 0)').trim()
    return [/^\(min-width: 0\)(\s+and\s+\(min-width: 0\))*$/.test(rest) ? 'all' : rest]
  })
  return queries.length === 0 ? null : queries.join(', ')
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
    highlighted.innerHTML = codeBlockHtml(text, language)
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
    return trimmed === '#' ? null : `#${ID_PREFIX}${decodedTarget(trimmed.slice(1))}`
  if (isExternal(trimmed))
    return trimmed
  if (hasScheme(trimmed))
    return null
  return hostPath(trimmed, place)
}

/** Where an image loads from: the web, at its address, or the Host, from the file at a path; null drops it. */
export function imageTarget(target: string, place: DocumentPlace): { url: string } | { path: string } | null {
  const trimmed = target.trim()
  if (isExternal(trimmed))
    return { url: trimmed }
  if (hasScheme(trimmed) || trimmed.startsWith('#'))
    return null
  const path = hostPath(trimmed, place)
  return path === null ? null : { path }
}

/** The Host path a relative or `/`-rooted target names, its query and fragment dropped. */
function hostPath(target: string, place: DocumentPlace): string | null {
  const file = decodedTarget(target.split(/[?#]/)[0] ?? '')
  if (file === '')
    return null
  return file.startsWith('/') ? joinPath(place.root, file) : joinPath(parentPath(place.path), file)
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
