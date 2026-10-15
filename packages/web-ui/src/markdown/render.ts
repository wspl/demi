import type { RendererObject, Token, Tokens } from 'marked'
import { markdownReader } from './reader'
import { baseName } from '@demicodes/utils'
import type { MarkdownRenderOptions, MessageAttachment, MessageFileTarget, MessageFiles } from './types'
import type { HostImage } from '../files/host-image'
import { codeBlockHtml } from './highlight'
import { WEB_LINK_REL, attachmentId, isHttpUrl, linkedCommit, messageFileTarget, messageImage } from './filePath'
import { codeFileTarget } from './code-paths'
import { linkedAgent } from '../agent/agent-links'
import type { CopiedStoredFile } from '../agent/message-input/copied-message'
import { KEEPS_WINDOWS } from '../ui/window-stack'
import { escapeHtml } from './html'
import { imagePlaceholderHtml } from './image-placeholder'
import { mathExtensions } from './math'
import { scrollingTable } from './table'
import { declaredSize, type PixelSize } from '../files/thumbnail'
import type { ShownMedium } from '../files/media-viewer'

// Parsing is synchronous, so the renderers read the options of the parse in flight instead of
// building a Marked instance (and re-registering KaTeX) per call.
let activeOptions: MarkdownRenderOptions | undefined
/** Links open around the renderer: an image inside one follows it instead of opening itself. */
let openLinks = 0

/** A link that leaves for the web, in a new tab. */
function webLink(url: string): string {
  return `<a href="${escapeHtml(url)}" target="_blank" rel="${WEB_LINK_REL}">`
}

/**
 * A link that opens a Host file through the `file` intent, at its line when
 * it names one. A link of text names the file with its icon first, the one
 * the file tree shows for it, which the page puts in its place
 * (`StreamedMarkdown`), so it reads as a file and not as a web page; a link
 * that starts with an icon, a commit's too (`commitLink`), is spaced from the
 * words around it (`icon-link`).
 */
function fileLink({ path, line }: MessageFileTarget, named: boolean, tight = false): string {
  const at = line === null ? '' : ` data-line="${line}"`
  if (!named)
    return `<a href="${escapeHtml(path)}" data-file-link${at}>`
  const icon = `<span class="message-link-icon" data-file-icon="${escapeHtml(baseName(path))}" aria-hidden="true"></span>`
  return `<a href="${escapeHtml(path)}" class="${iconLinkClass(tight)}" data-file-link${at}>${icon}`
}

/**
 * A link that opens a commit's changes, with the commit's icon first, which
 * the page puts in its place (`StreamedMarkdown`) as it does a file's, and
 * spaced as a file's link is (`icon-link`).
 */
function commitLink(commit: string, href: string, body: string, tight: boolean): string {
  const icon = '<span class="message-link-icon" data-commit-icon aria-hidden="true"></span>'
  // The icon stays with the first word of the text on its line, as an agent's mark does.
  const lead = firstWord(body.slice(0, body.search(/<|$/)))
  return `<a href="${escapeHtml(href)}" class="${iconLinkClass(tight)}" data-commit-link="${escapeHtml(commit)}">`
    + `<span class="icon-lead">${icon}${lead}</span>${body.slice(lead.length)}</a>`
}

/** The classes of a link that starts with an icon, `tight` when ASCII punctuation follows it, which keeps no space after its name. */
function iconLinkClass(tight: boolean): string {
  return tight ? 'icon-link icon-link-tight' : 'icon-link'
}

/** Punctuation that stands right after the word before it in Latin text, which a link's own space must not part from it. */
const CLOSING_PUNCTUATION_RE = /^[.,;:!?)\]}]/

/**
 * Marks each link that starts with an icon and is followed at once by
 * ASCII punctuation, so that it keeps no space after its name: "main agent;"
 * reads as it was written, where the 0.2em of space an icon link keeps would
 * show "main agent ;". Full-width punctuation carries its own space and
 * keeps the link's.
 */
function tightenLinks(tokens: Token[]): void {
  const mark = (inline: readonly Token[]) => inline.forEach((part, index) => {
    const next = inline[index + 1]
    if (part.type === 'link' && next?.type === 'text' && CLOSING_PUNCTUATION_RE.test(next.raw))
      tightLinks.add(part)
  })
  // A walk gives each token, but a table's cells hold their inline tokens apart from it.
  agentMarked.walkTokens(tokens, (token) => {
    if (token.type === 'table') {
      for (const cell of [...token.header, ...token.rows.flat()])
        mark(cell.tokens)
    } else if ('tokens' in token && Array.isArray(token.tokens)) {
      mark(token.tokens)
    }
  })
}

/** The links `tightenLinks` marked, by token. */
const tightLinks = new WeakSet<Token>()

/**
 * A link to agent `number` of the conversation (`subagents.md` § Linking an
 * agent), around its text: the agent's mark, which the page puts in its
 * place with the agent's live state, then the text, and a click opens it in
 * the Agents window. A number the conversation does not have shows the text
 * alone; the page's tip says so. Outside a conversation, the text alone.
 * It starts with an icon, so it is spaced as a file's link is
 * (`icon-link`), and it is a control of the Agents window, as the window's
 * chip is: a press on it is not one outside the window (`KEEPS_WINDOWS`),
 * so a click when the window shows its tab closes the window.
 */
function agentLink(number: number, body: string, tight: boolean): string {
  const agents = activeOptions?.agents
  if (!agents)
    return body
  if (!agents.find(number))
    return `<span class="agent-link-missing" data-agent-link="${number}">${body}</span>`
  // A button element lays out as one block, so a long name would move whole
  // to a line of its own: the link is inline text that acts as a button.
  const lead = firstWord(body.slice(0, body.search(/<|$/)))
  return `<span role="button" tabindex="0" class="agent-link ${iconLinkClass(tight)}" data-agent-link="${number}" ${KEEPS_WINDOWS}>`
    + `<span class="icon-lead"><span class="agent-link-mark" data-agent-mark="${number}"></span>${lead}</span>`
    + `${body.slice(lead.length)}</span>`
}

/**
 * The first word of `html`, text that holds no tag, which an agent link's
 * mark keeps with it on its line (a word joiner holds no inline box to the
 * text after it): a word as the language breaks words, so a long name in a
 * language written without spaces still wraps after it. An entity is never
 * cut.
 */
function firstWord(html: string): string {
  const [first] = new Intl.Segmenter(undefined, { granularity: 'word' }).segment(html)
  const word = first?.segment ?? ''
  const entity = word.lastIndexOf('&')
  if (entity < 0 || word.includes(';', entity))
    return word
  const end = html.indexOf(';', entity)
  return end < 0 ? html : html.slice(0, end + 1)
}

/**
 * The attribute of an attachment's link that holds the file as Demi's own
 * form of a copy holds it, which a drag of the link into a composer carries
 * (`copied-message.ts` § `attachmentDrag`).
 */
export const ATTACHMENT_FILE_ATTRIBUTE = 'data-attachment-file'

/** The attribute of a Host image that names its file's path, which a click on it shows in the viewer (`openMessageLink`). */
export const HOST_FILE_ATTRIBUTE = 'data-host-file'

/** An attachment as Demi's own form of a copy holds a stored file. */
function attachmentFile(attachment: MessageAttachment): CopiedStoredFile {
  return { type: 'stored', name: attachment.name, sha256: attachment.sha256, mediaType: attachment.mediaType }
}

/**
 * A link that opens an attachment: an image or a video large in the page's
 * viewer, where a video plays, and any other file as a download under its
 * name. One that opens the viewer is the viewer's control: a press on it
 * does not close the viewer, which shows the medium in place of the one it
 * shows (`KEEPS_WINDOWS`).
 */
function attachmentLink(attachment: MessageAttachment, attributes = ''): string {
  const url = escapeHtml(attachment.url)
  const name = escapeHtml(attachment.name)
  const file = ` ${ATTACHMENT_FILE_ATTRIBUTE}="${escapeHtml(JSON.stringify(attachmentFile(attachment)))}"`
  if (attachment.mediaType.startsWith('image/'))
    return `<a href="${url}" data-attachment-image="${name}"${file} ${KEEPS_WINDOWS}${attributes}>`
  if (attachment.mediaType.startsWith('video/'))
    return `<a href="${url}" data-attachment-video="${name}"${file} ${KEEPS_WINDOWS}${attributes}>`
  return `<a href="${url}" download="${name}"${file}${attributes}>`
}

/**
 * The size an attachment's record carries, which a medium's tile is taken
 * from before its bytes arrive, as attributes that `media-run.ts` reads
 * once a render is in the page and its width is known.
 */
function sizeAttributes(size: PixelSize | null): string {
  return size ? ` data-width="${size.width}" data-height="${size.height}"` : ''
}

/**
 * An attachment's video as its first frame with a play mark over it, which
 * a click opens in the viewer to play; inside a link, it follows the link.
 * It is a tile, as an image is.
 */
function attachmentVideo(attachment: MessageAttachment, alt: string, title: string): string {
  const video = `<video src="${escapeHtml(attachment.url)}" muted playsinline preload="metadata" aria-label="${alt}"${title}${sizeAttributes(declaredSize(attachment))}></video>`
  const mark = '<span class="media-play-mark" aria-hidden="true"></span>'
  const wrapper = ' class="message-video"'
  if (openLinks > 0)
    return `<span${wrapper}>${video}${mark}</span>`
  return `${attachmentLink(attachment, wrapper)}${video}${mark}</a>`
}

/**
 * How a link or an image that names attachment `id` shows, around `body`,
 * its text or alt text: through `found` once the attachment is known; its
 * text with a line that says so for a number the conversation does not
 * have; its text alone while the page asks, or cannot.
 */
function attachmentTarget(
  id: string,
  body: string,
  found: (attachment: MessageAttachment) => string,
): string {
  const lookup = activeOptions?.files?.attachment?.(id)
  switch (lookup?.state) {
    case 'found':
      return found(lookup.attachment)
    case 'missing':
      return `${body} <span class="attachment-missing">No attachment ${escapeHtml(id)} in this conversation</span>`
    default:
      return body
  }
}

/**
 * Where an image loads from: its URL, or a Host image's file at the version
 * the page keeps of it, with the path the click shows it by; `loading`
 * while a Host image's file is read. Null shows its alt text.
 */
function imageSource(target: string): { src: string; path: string | null } | 'loading' | null {
  const image = messageImage(target, activeOptions?.files)
  if (!image)
    return null
  if ('url' in image)
    return { src: image.url, path: null }
  const shown: HostImage = activeOptions?.hostImages?.get(image.path) ?? { state: 'loading' }
  if (shown.state !== 'shown')
    return shown.state === 'loading' ? 'loading' : null
  return { src: shown.src, path: image.path }
}

/**
 * An attachment an image names: an image that a click shows large, a video's
 * first frame that a click plays large, each a tile whose box its record's
 * size gives before its bytes arrive, and any other file as a link that
 * downloads it, named by the alt text.
 */
function attachmentMedium(attachment: MessageAttachment, token: Tokens.Image): string {
  const alt = escapeHtml(token.text)
  const title = token.title ? ` title="${escapeHtml(token.title)}"` : ''
  if (attachment.mediaType.startsWith('video/'))
    return attachmentVideo(attachment, alt, title)
  if (!attachment.mediaType.startsWith('image/'))
    return openLinks > 0 ? alt : `${attachmentLink(attachment)}${alt}</a>`
  const image = `<img src="${escapeHtml(attachment.url)}" alt="${alt}"${title}${sizeAttributes(declaredSize(attachment))} />`
  return openLinks > 0 ? image : `${attachmentLink(attachment)}${image}</a>`
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
    return codeBlockHtml(text, lang ?? '')
  },
  // Inline code that names a file the Host has opens it, in its code look
  // (`file-previews.md` § Files named in messages).
  codespan({ text }) {
    const code = `<code>${escapeHtml(text)}</code>`
    const files = activeOptions?.files
    const existing = activeOptions?.codeFiles
    if (!files?.open || !existing || openLinks > 0)
      return code
    const target = codeFileTarget(text, files.cwd)
    return target && existing.has(target.path) ? `${fileLink(target, true)}${code}</a>` : code
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
    const attachment = attachmentId(token.href)
    if (attachment !== null)
      return attachmentTarget(attachment, body, (found) => `${attachmentLink(found)}${body}</a>`)
    const agent = linkedAgent(token.href)
    if (agent !== null)
      return agentLink(agent, body, tightLinks.has(token))
    // A commit opens its changes; nobody to open it, its text alone.
    const commit = linkedCommit(token.href)
    if (commit !== null)
      return activeOptions?.files?.openCommit ? `${commitLink(commit, token.href, body, tightLinks.has(token))}` : body
    // A file nobody opens is named as text.
    const files = activeOptions?.files
    const target = files?.open ? messageFileTarget(token.href, files) : null
    if (target === null)
      return body
    // A picture that links a file shows itself, not the file's icon.
    const named = !(token.tokens ?? []).some((part) => part.type === 'image')
    return `${fileLink(target, named, tightLinks.has(token))}${body}</a>`
  },
  image(token) {
    const attachment = attachmentId(token.href)
    if (attachment !== null)
      return attachmentTarget(attachment, escapeHtml(token.text), (found) => attachmentMedium(found, token))
    const source = imageSource(token.href)
    // A target that loads nothing is words; a Host image whose file is being
    // read or is not there keeps its place with its alt text in a box.
    if (source === null && messageImage(token.href, activeOptions?.files) === null)
      return escapeHtml(token.text)
    if (source === null || source === 'loading')
      return imagePlaceholderHtml(token.text)
    const title = token.title ? ` title="${escapeHtml(token.title)}"` : ''
    // A Host image names its file, which the viewer a click opens follows.
    const file = source.path === null ? '' : ` ${HOST_FILE_ATTRIBUTE}="${escapeHtml(source.path)}"`
    return `<img src="${escapeHtml(source.src)}" alt="${escapeHtml(token.text)}"${title}${file} />`
  },
}

/** Whether an inline token is only white space: a space or a line break. */
function isWhiteSpace(token: Token): boolean {
  return token.type === 'br' || (token.type === 'text' && token.raw.trim() === '')
}

/**
 * Whether an image token shows a picture, or a video's first frame: an image
 * that loads, or an attachment that is an image or a video, also while the
 * page still asks for it. One that never shows, which leaves its alt text or
 * a download link, is words.
 */
function showsMedium(token: Token): boolean {
  if (token.type !== 'image')
    return false
  const href: string = token.href
  const id = attachmentId(href)
  if (id === null)
    return imageSource(href) !== null
  const lookup = activeOptions?.files?.attachment?.(id)
  if (!lookup || lookup.state === 'missing')
    return false
  return lookup.state !== 'found' || /^(?:image|video)\//.test(lookup.attachment.mediaType)
}

/** Whether an inline token is a medium of the message: an image that shows, or a link around one and nothing else. */
function isMedium(token: Token): boolean {
  if (showsMedium(token))
    return true
  if (token.type !== 'link')
    return false
  const inside = (token.tokens ?? []).filter((part) => !isWhiteSpace(part))
  return inside.length === 1 && inside[0] !== undefined && showsMedium(inside[0])
}

/**
 * The block a message's media stand in, on a line of their own: one medium
 * alone, or a run of two or more side by side
 * (`file-previews.md` § Files named in messages).
 */
const MEDIA = 'messageMedia'

function mediaBlock(media: Token[]): Token {
  return { type: MEDIA, raw: media.map((medium) => medium.raw).join('\n'), tokens: media }
}

/** `token` without the white space at its start or its end, when it is text. */
function trimmedText(token: Token, edge: 'start' | 'end'): Token {
  if (token.type !== 'text')
    return token
  const trim = (value: string) => (edge === 'start' ? value.trimStart() : value.trimEnd())
  return { ...token, raw: trim(token.raw), text: trim(token.text) }
}

/** The words of a paragraph between its media, without the white space and line breaks at their edges. */
function trimmedWords(words: Token[]): Token[] {
  const first = words.findIndex((token) => !isWhiteSpace(token))
  if (first < 0)
    return []
  const last = words.findLastIndex((token) => !isWhiteSpace(token))
  const kept = words.slice(first, last + 1)
  kept[0] = trimmedText(kept[0]!, 'start')
  kept[kept.length - 1] = trimmedText(kept.at(-1)!, 'end')
  return kept
}

/**
 * A paragraph, or a tight list item's text, split at its media: the words
 * before them, the media that follow each other with nothing but white space
 * between them, the words after, each a block of its own, so a picture always
 * stands on its own line, even on the line after a list item's words. Null
 * for one with no media.
 */
function splitAtMedia(block: Tokens.Paragraph | Tokens.Text | Tokens.Generic): Token[] | null {
  const inline = block.tokens ?? []
  if (!inline.some(isMedium))
    return null
  const parts: Token[] = []
  let words: Token[] = []
  let media: Token[] = []
  const endWords = () => {
    const kept = trimmedWords(words)
    if (kept.length > 0) {
      const raw = kept.map((token) => token.raw).join('')
      parts.push({ type: block.type, raw, text: raw, tokens: kept })
    }
    words = []
  }
  const endMedia = () => {
    if (media.length > 0)
      parts.push(mediaBlock(media))
    media = []
  }
  for (const token of inline) {
    if (isMedium(token)) {
      endWords()
      media.push(token)
    } else if (media.length === 0 || !isWhiteSpace(token)) {
      endMedia()
      words.push(token)
    }
  }
  endWords()
  endMedia()
  return parts
}

/** Joins media blocks that follow each other, across the blank lines between them, into one run. */
function joinMediaRuns(blocks: Token[]): void {
  for (let index = 0; index < blocks.length; index += 1) {
    const first = blocks[index]
    if (first?.type !== MEDIA)
      continue
    const media = [...(first.tokens ?? [])]
    let last = index
    for (let next = index + 1; next < blocks.length; next += 1) {
      const block = blocks[next]
      if (block?.type === 'space')
        continue
      if (block?.type !== MEDIA)
        break
      media.push(...(block.tokens ?? []))
      last = next
    }
    if (last > index)
      blocks.splice(index, last - index + 1, mediaBlock(media))
  }
}

/**
 * Gives each of a message's media, and each run of them, a block of its own,
 * in the message and in its lists and quotes: a lone image stands on its
 * own line, and images that follow each other stand side by side, whatever
 * words come before the first of them in its paragraph; any other text, a
 * list or a heading ends a run (`file-previews.md` § Files named in messages).
 */
function separateMedia(blocks: Token[]): void {
  for (let index = 0; index < blocks.length; index += 1) {
    const block = blocks[index]
    if (block?.type === 'list') {
      for (const item of block.items)
        separateMedia(item.tokens)
    } else if (block?.type === 'blockquote') {
      separateMedia(block.tokens ?? [])
    } else if (block?.type === 'paragraph' || block?.type === 'text') {
      const parts = splitAtMedia(block)
      if (parts) {
        blocks.splice(index, 1, ...parts)
        index += parts.length - 1
      }
    }
  }
  joinMediaRuns(blocks)
}

const agentMarked = markdownReader({ breaks: true, renderer: messageRenderer }, scrollingTable)
// LaTeX math, rendered to self-contained HTML (KaTeX CSS is loaded by web-ui's base
// stylesheet); `throwOnError` keeps malformed math from blowing up the whole message.
agentMarked.use({ extensions: mathExtensions({ throwOnError: false, output: 'html' }) })
agentMarked.use({
  hooks: {
    processAllTokens(tokens) {
      tightenLinks(tokens)
      separateMedia(tokens)
      return tokens
    },
  },
  extensions: [{
    name: MEDIA,
    // Each medium is a tile (`media-run.ts` sizes it once the message's
    // width is known), lone or side by side with the others of its run.
    renderer(token) {
      const media = token.tokens ?? []
      const block = media.length === 1 ? 'media-lone' : 'media-run'
      return `<p class="${block}">${media.map((medium) => this.parser.parseInline([medium])).join('')}</p>\n`
    },
  }],
})

/** The text of each inline code span of an agent's message, in order, at any depth. */
export function messageCodeSpans(src: string): string[] {
  const spans: string[] = []
  agentMarked.walkTokens(agentMarked.lexer(src), (token) => {
    if (token.type === 'codespan')
      spans.push(token.text)
  })
  return spans
}

/**
 * The pictures a message shows that open in the media viewer, in order:
 * each image that loads and each attachment that is an image or a video,
 * named and sized as a click on it names and sizes it. An image inside a
 * link follows the link and is not among them; neither is one still being
 * asked for, or one that shows its alt text.
 */
export function messageMedia(src: string, files: MessageFiles | undefined): ShownMedium[] {
  if (!src.includes('!['))
    return []
  const tokens = agentMarked.lexer(src)
  const linked = new Set<Token>()
  agentMarked.walkTokens(tokens, (token) => {
    if (token.type === 'link')
      agentMarked.walkTokens(token.tokens ?? [], (inside) => void linked.add(inside))
  })
  const media: ShownMedium[] = []
  agentMarked.walkTokens(tokens, (token) => {
    if (token.type !== 'image' || linked.has(token))
      return
    const id = attachmentId(token.href)
    if (id === null) {
      const image = messageImage(token.href, files)
      const name = token.text || 'image'
      if (image && 'url' in image)
        media.push({ kind: 'image', src: image.url, name })
      // A Host image is its file, which the viewer shows at the version the page keeps.
      else if (image && files?.contents)
        media.push({ kind: 'image', file: { path: image.path, contents: files.contents }, name })
      return
    }
    const lookup = files?.attachment?.(id)
    if (lookup?.state !== 'found')
      return
    const { attachment } = lookup
    const kind = attachment.mediaType.startsWith('image/') ? 'image' : attachment.mediaType.startsWith('video/') ? 'video' : null
    if (kind)
      media.push({ kind, src: attachment.url, name: attachment.name, size: declaredSize(attachment) })
  })
  return media
}

/** The target of each image of an agent's message, in order, at any depth. */
export function messageImageTargets(src: string): string[] {
  const targets: string[] = []
  agentMarked.walkTokens(agentMarked.lexer(src), (token) => {
    if (token.type === 'image')
      targets.push(token.href)
  })
  return targets
}

/**
 * A line that opens display math as the math extension reads it: `$$` or `$`
 * alone. The extension takes a block as math only once its closing line has
 * arrived, so before that its lines lex as text.
 */
const MATH_OPENER = /^\${1,2}\n/

/**
 * Where the blocks of `src`, the text of a streaming reply, end that no text
 * arriving after it can change (`file-previews.md`, a reply that streams
 * renders a block at a time), as offsets into it in order. The blocks are
 * the top-level blocks the message's own reader finds, so a blank line
 * inside a fence, a list item or an indented code block never ends one,
 * and a fence still open runs to the end. They are read from the lines
 * that have ended, since the rest of a line can change what the lines
 * before it are: `1. a`, a blank line and `2` are a list and a paragraph
 * until `. b` makes them one list, and `x | y` above `- | -` reads as a
 * table's head until the line goes on as text. Every block before the last
 * is settled, except:
 * - a block that opens display math before its closing line (`MATH_OPENER`),
 *   and every block after it;
 * - a raw HTML block, and every block after it: only the whole text nests
 *   what follows `<details>` inside it.
 */
export function messageBlockEnds(src: string): number[] {
  const blocks: { start: number; live: boolean }[] = []
  let offset = 0
  for (const token of new agentMarked.Lexer(agentMarked.defaults).blockTokens(src.slice(0, src.lastIndexOf('\n') + 1))) {
    if (token.type !== 'space')
      blocks.push({ start: offset, live: token.type === 'html' || (token.type !== 'blockKatex' && MATH_OPENER.test(token.raw)) })
    offset += token.raw.length
  }
  let last = blocks.length - 1
  const live = blocks.findIndex((block) => block.live)
  if (live >= 0)
    last = Math.min(last, live)
  return blocks.slice(1, Math.max(1, last + 1)).map((block) => block.start)
}

/** An agent's message: GitHub Flavored Markdown with math. */
export function renderMarkdown(src: string, options?: MarkdownRenderOptions): string {
  activeOptions = options
  try {
    return agentMarked.parse(src, { async: false })
  } finally {
    activeOptions = undefined
  }
}
