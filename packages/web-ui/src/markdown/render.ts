import type { RendererObject, Token, Tokens } from 'marked'
import { markdownReader } from './reader'
import { baseName } from '@demicodes/utils'
import type { MarkdownRenderOptions, MessageAttachment, MessageFileTarget, MessageFiles } from './types'
import { codeBlockHtml } from './highlight'
import { attachmentId, isHttpUrl, messageFileTarget, messageImage } from './filePath'
import { codeFileTarget } from './code-paths'
import { linkedAgent } from '../agent/agent-links'
import { KEEPS_WINDOWS } from '../ui/window-stack'
import { escapeHtml } from './html'
import { mathExtensions } from './math'
import { scrollingTable } from './table'
import { declaredSize, THUMBNAIL_HEIGHT, thumbnailBox, thumbnailPlaceHeight, type PixelSize, type ThumbnailKind } from '../files/thumbnail'

// Parsing is synchronous, so the renderers read the options of the parse in flight instead of
// building a Marked instance (and re-registering KaTeX) per call.
let activeOptions: MarkdownRenderOptions | undefined
/** Links open around the renderer: an image inside one follows it instead of opening itself. */
let openLinks = 0
/**
 * Inside a run of a message's media, where each one is a thumbnail, the
 * height of the place the medium being rendered takes in the run's row;
 * null outside a run.
 */
let runPlace: number | null = null

/** A link that leaves for the web, in a new tab. */
function webLink(url: string): string {
  return `<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer">`
}

/**
 * A link that opens a Host file through the `file` intent, at its line when
 * it names one. A link of text names the file with its icon first, the one
 * the file tree shows for it, which the page puts in its place
 * (`StreamedMarkdown`), so it reads as a file and not as a web page; a link
 * that starts with an icon is spaced from the words around it
 * (`icon-link`).
 */
function fileLink({ path, line }: MessageFileTarget, named: boolean): string {
  const at = line === null ? '' : ` data-line="${line}"`
  if (!named)
    return `<a href="${escapeHtml(path)}" data-file-link${at}>`
  const icon = `<span class="message-file-icon" data-file-icon="${escapeHtml(baseName(path))}" aria-hidden="true"></span>`
  return `<a href="${escapeHtml(path)}" class="icon-link" data-file-link${at}>${icon}`
}

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
function agentLink(number: number, body: string): string {
  const agents = activeOptions?.agents
  if (!agents)
    return body
  if (!agents.find(number))
    return `<span class="agent-link-missing" data-agent-link="${number}">${body}</span>`
  // A button element lays out as one block, so a long name would move whole
  // to a line of its own: the link is inline text that acts as a button.
  const lead = firstWord(body.slice(0, body.search(/<|$/)))
  return `<span role="button" tabindex="0" class="agent-link icon-link" data-agent-link="${number}" ${KEEPS_WINDOWS}>`
    + `<span class="agent-link-lead"><span class="agent-link-mark" data-agent-mark="${number}"></span>${lead}</span>`
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
 * A link that opens an attachment: an image or a video large in the page's
 * viewer, where a video plays, and any other file as a download under its
 * name. One that opens the viewer is the viewer's control: a press on it
 * does not close the viewer, which shows the medium in place of the one it
 * shows (`KEEPS_WINDOWS`).
 */
function attachmentLink(attachment: MessageAttachment, attributes = ''): string {
  const url = escapeHtml(attachment.url)
  const name = escapeHtml(attachment.name)
  if (attachment.mediaType.startsWith('image/'))
    return `<a href="${url}" data-attachment-image="${name}" ${KEEPS_WINDOWS}${attributes}>`
  if (attachment.mediaType.startsWith('video/'))
    return `<a href="${url}" data-attachment-video="${name}" ${KEEPS_WINDOWS}${attributes}>`
  return `<a href="${url}" download="${name}"${attributes}>`
}

/**
 * The size an attachment's record carries, which a medium's box is taken
 * from before its bytes arrive, as attributes that `media-run.ts` reads
 * again once a render is in the page.
 */
function sizeAttributes(size: PixelSize | null): string {
  return size ? ` data-width="${size.width}" data-height="${size.height}"` : ''
}

/**
 * A medium in a run as a thumbnail of `size`, known before its bytes arrive:
 * its box as its inline style, and the height of its place in the run.
 */
function runThumbnail(size: PixelSize | null, kind: ThumbnailKind): string {
  const box = thumbnailBox(size, kind)
  runPlace = thumbnailPlaceHeight(size, kind)
  return ` style="width: ${box.width}px; height: ${box.height}px"`
}

/**
 * A lone video's first frame, sized as a lone image is, by the proportions
 * and the width its record carries: 16:9 and the message's width while they
 * are unknown (`media-run.ts` sets them once its first frame arrives).
 */
function loneVideoStyle(size: PixelSize | null): string {
  if (!size)
    return ''
  return ` style="--media-ratio: ${size.width / size.height}; --media-width: ${size.width}px"`
}

/**
 * An attachment's video as its first frame with a play mark over it, which
 * a click opens in the viewer to play; inside a link, it follows the link.
 * In a run it is a thumbnail.
 */
function attachmentVideo(attachment: MessageAttachment, alt: string, title: string): string {
  const size = declaredSize(attachment)
  const inRun = runPlace !== null
  const frame = inRun ? runThumbnail(size, 'video') : ''
  const video = `<video src="${escapeHtml(attachment.url)}" muted playsinline preload="metadata" aria-label="${alt}"${title}${sizeAttributes(size)}${frame}></video>`
  const mark = '<span class="media-play-mark" aria-hidden="true"></span>'
  const wrapper = ` class="message-video"${inRun ? '' : loneVideoStyle(size)}`
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
 * Where an image loads from, and the link a click on it follows to show it
 * whole. Null shows its alt text.
 */
function imageSource(target: string): { src: string; link: string | null } | null {
  const image = messageImage(target, activeOptions?.files)
  if (!image)
    return null
  if (!image.opens)
    return { src: image.src, link: null }
  return { src: image.src, link: 'web' in image.opens ? webLink(image.opens.web) : fileLink({ path: image.opens.file, line: null }, false) }
}

/**
 * An attachment an image names: an image that a click shows large, a video's
 * first frame that a click plays large, each at an image's bounds and in a
 * run as a thumbnail whose box its record's size gives before its bytes
 * arrive, and any other file as a link that downloads it, named by the alt
 * text.
 */
function attachmentMedium(attachment: MessageAttachment, token: Tokens.Image): string {
  const alt = escapeHtml(token.text)
  const title = token.title ? ` title="${escapeHtml(token.title)}"` : ''
  if (attachment.mediaType.startsWith('video/'))
    return attachmentVideo(attachment, alt, title)
  if (!attachment.mediaType.startsWith('image/'))
    return openLinks > 0 ? alt : `${attachmentLink(attachment)}${alt}</a>`
  const size = declaredSize(attachment)
  const box = runPlace !== null ? runThumbnail(size, 'image') : ''
  const image = `<img src="${escapeHtml(attachment.url)}" alt="${alt}"${title}${sizeAttributes(size)}${box} />`
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
      return agentLink(agent, body)
    // A file nobody opens is named as text.
    const files = activeOptions?.files
    const target = files?.open ? messageFileTarget(token.href, files.cwd) : null
    if (target === null)
      return body
    // A picture that links a file shows itself, not the file's icon.
    const named = !(token.tokens ?? []).some((part) => part.type === 'image')
    return `${fileLink(target, named)}${body}</a>`
  },
  image(token) {
    const attachment = attachmentId(token.href)
    if (attachment !== null)
      return attachmentTarget(attachment, escapeHtml(token.text), (found) => attachmentMedium(found, token))
    const source = imageSource(token.href)
    if (source === null)
      return escapeHtml(token.text)
    const title = token.title ? ` title="${escapeHtml(token.title)}"` : ''
    const box = runPlace !== null ? runThumbnail(null, 'image') : ''
    const image = `<img src="${escapeHtml(source.src)}" alt="${escapeHtml(token.text)}"${title}${box} />`
    return openLinks > 0 || source.link === null ? image : `${source.link}${image}</a>`
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
    return messageImage(href, activeOptions?.files) !== null
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
      separateMedia(tokens)
      return tokens
    },
  },
  extensions: [{
    name: MEDIA,
    renderer(token) {
      const media = token.tokens ?? []
      if (media.length === 1)
        return `<p class="media-lone">${this.parser.parseInline(media)}</p>\n`
      // In a run, each medium's place is as tall as its thumbnail's renderer
      // says; one that shows as text, while its attachment is asked for,
      // keeps the place of a medium whose size is unknown.
      const items: string[] = []
      try {
        for (const medium of media) {
          runPlace = THUMBNAIL_HEIGHT
          const html = this.parser.parseInline([medium])
          items.push(`<span style="height: ${runPlace}px">${html}</span>`)
        }
      } finally {
        runPlace = null
      }
      return `<p class="media-run">${items.join('')}</p>\n`
    },
  }],
})

/**
 * A medium cut off at the end of streamed text, after what holding back an
 * open link leaves: the `!` of an image, or a link still open around a whole
 * image.
 */
const PARTIAL_MEDIUM = /(?:\[?!|\[!\[[^\]]*\]\([^)]*\)\](?:\([^)]*)?)$/

/** The media a message's text ends with, in the innermost list item or quote it ends in; none when it ends with words. */
function trailingMedia(blocks: Token[]): Token[] {
  separateMedia(blocks)
  let inner = blocks
  for (;;) {
    const last = inner.findLast((block) => block.type !== 'space')
    if (last?.type === MEDIA)
      return last.tokens ?? []
    if (last?.type === 'list')
      inner = last.items.at(-1)?.tokens ?? []
    else if (last?.type === 'blockquote')
      inner = last.tokens ?? []
    else
      return []
  }
}

/**
 * Streamed text without the lone image at its end: whether that image stands
 * alone or starts a run is decided only by what follows it, so it shows once
 * that arrives, or once the message is complete, and never shows large first
 * and then shrinks into a row. An image that joins a run already shown shows
 * at once. `files` are the conversation's, which tell which images show.
 */
export function holdUndecidedMedium(text: string, files?: MessageFiles): string {
  const settled = text.replace(PARTIAL_MEDIUM, '')
  // Media end with the `)` of the last one.
  if (!/\)\s*$/.test(settled))
    return text
  activeOptions = { files }
  let media: Token[]
  try {
    media = trailingMedia(agentMarked.lexer(settled))
  } finally {
    activeOptions = undefined
  }
  const raw = media.length === 1 ? media[0]?.raw : undefined
  const end = settled.trimEnd()
  return raw !== undefined && end.endsWith(raw) ? end.slice(0, end.length - raw.length) : text
}

/** The text of each inline code span of an agent's message, in order, at any depth. */
export function messageCodeSpans(src: string): string[] {
  const spans: string[] = []
  agentMarked.walkTokens(agentMarked.lexer(src), (token) => {
    if (token.type === 'codespan')
      spans.push(token.text)
  })
  return spans
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
