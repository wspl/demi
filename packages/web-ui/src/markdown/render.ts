import { Marked, type RendererObject, type Token, type Tokens } from 'marked'
import markedKatex from 'marked-katex-extension'
import type { MarkdownRenderOptions, MessageAttachment } from './types'
import { codeToHtml } from './highlight'
import { attachmentId, isHttpUrl, messageHostPath, messageImage } from './filePath'
import { escapeHtml } from './html'
import { declaredSize, THUMBNAIL_HEIGHT, thumbnailBox, type PixelSize, type ThumbnailKind } from '../files/thumbnail'

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
/** Whether the renderer is inside a run of a message's media, where each one is a thumbnail. */
let inRun = false

/** A link that leaves for the web, in a new tab. */
function webLink(url: string): string {
  return `<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer">`
}

/** A link that opens a Host file through the `file` intent. */
function fileLink(path: string): string {
  return `<a href="${escapeHtml(path)}" data-file-link>`
}

/**
 * A link that opens an attachment: an image or a video large in the page's
 * viewer, where a video plays, and any other file as a download under its
 * name.
 */
function attachmentLink(attachment: MessageAttachment, attributes = ''): string {
  const url = escapeHtml(attachment.url)
  const name = escapeHtml(attachment.name)
  if (attachment.mediaType.startsWith('image/'))
    return `<a href="${url}" data-attachment-image="${name}"${attributes}>`
  if (attachment.mediaType.startsWith('video/'))
    return `<a href="${url}" data-attachment-video="${name}"${attributes}>`
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

/** A thumbnail's box, as the inline style of a medium in a run. */
function thumbnailStyle(size: PixelSize | null, kind: ThumbnailKind): string {
  const box = thumbnailBox(size, kind)
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
  const frame = inRun ? thumbnailStyle(size, 'video') : ''
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
  const box = inRun ? thumbnailStyle(size, 'image') : ''
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
    const attachment = attachmentId(token.href)
    if (attachment !== null)
      return attachmentTarget(attachment, body, (found) => `${attachmentLink(found)}${body}</a>`)
    // A file nobody opens is named as text.
    const path = activeOptions?.files?.open ? hostPath(token.href) : null
    return path === null ? body : `${fileLink(path)}${body}</a>`
  },
  image(token) {
    const attachment = attachmentId(token.href)
    if (attachment !== null)
      return attachmentTarget(attachment, escapeHtml(token.text), (found) => attachmentMedium(found, token))
    const source = imageSource(token.href)
    if (source === null)
      return escapeHtml(token.text)
    const title = token.title ? ` title="${escapeHtml(token.title)}"` : ''
    const box = inRun ? thumbnailStyle(null, 'image') : ''
    const image = `<img src="${escapeHtml(source.src)}" alt="${escapeHtml(token.text)}"${title}${box} />`
    return openLinks > 0 || source.link === null ? image : `${source.link}${image}</a>`
  },
}

/** Whether an inline token is only white space: a space or a line break. */
function isWhiteSpace(token: Token): boolean {
  return token.type === 'br' || (token.type === 'text' && token.raw.trim() === '')
}

/** Whether an inline token is an image, or a link around one image and nothing else. */
function isMedium(token: Token): boolean {
  if (token.type === 'image')
    return true
  if (token.type !== 'link')
    return false
  const inside = (token.tokens ?? []).filter((part) => !isWhiteSpace(part))
  return inside.length === 1 && inside[0]?.type === 'image'
}

/**
 * The images a block holds when it is a paragraph of images with nothing but
 * white space between them; null for any other block.
 */
function paragraphMedia(token: Token): Token[] | null {
  if (token.type !== 'paragraph')
    return null
  const media: Token[] = []
  for (const part of token.tokens ?? []) {
    if (isMedium(part))
      media.push(part)
    else if (!isWhiteSpace(part))
      return null
  }
  return media.length > 0 ? media : null
}

/**
 * The images of the paragraphs of images that follow each other from block
 * `start`, across the blank lines between them, and the index after the last
 * of those paragraphs; none when block `start` is not one.
 */
function mediaFrom(blocks: Token[], start: number): { media: Token[]; end: number } {
  const media: Token[] = []
  let end = start
  for (let index = start; index < blocks.length; index += 1) {
    const block = blocks[index]
    if (block === undefined)
      break
    if (block.type === 'space' && media.length > 0)
      continue
    const found = paragraphMedia(block)
    if (found === null)
      break
    media.push(...found)
    end = index + 1
  }
  return { media, end }
}

/** The block a run of images becomes: two or more of them, in one paragraph or in several that follow each other. */
const MEDIA_RUN = 'mediaRun'

/**
 * Turns each run of a message's images into one block, which stands them side
 * by side; a lone image stays in its paragraph
 * (`file-previews.md` § Files named in messages).
 */
function groupMediaRuns(blocks: Token[]): void {
  for (let index = 0; index < blocks.length; index += 1) {
    const { media, end } = mediaFrom(blocks, index)
    if (media.length < 2)
      continue
    const raw = blocks.slice(index, end).map((block) => block.raw).join('')
    blocks.splice(index, end - index, { type: MEDIA_RUN, raw, tokens: media })
  }
}

const agentMarked = new Marked({ gfm: true, breaks: true, renderer: messageRenderer })
agentMarked.use(katexExtension)
agentMarked.use({
  hooks: {
    processAllTokens(tokens) {
      groupMediaRuns(tokens)
      return tokens
    },
  },
  extensions: [{
    name: MEDIA_RUN,
    renderer(token) {
      inRun = true
      let items: string[]
      try {
        items = (token.tokens ?? []).map((medium) => `<span style="height: ${THUMBNAIL_HEIGHT}px">${this.parser.parseInline([medium])}</span>`)
      } finally {
        inRun = false
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

/**
 * Streamed text without the lone image at its end: whether that image stands
 * alone or starts a run is decided only by what follows it, so it shows once
 * that arrives, or once the message is complete, and never shows large first
 * and then shrinks into a row. An image that joins a run already shown shows
 * at once.
 */
export function holdUndecidedMedium(text: string): string {
  const settled = text.replace(PARTIAL_MEDIUM, '')
  // A paragraph of images ends with the `)` of its last one.
  if (!/\)\s*$/.test(settled))
    return text
  const blocks = agentMarked.lexer(settled)
  let start = blocks.length
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    const block = blocks[index]
    if (block === undefined)
      break
    if (block.type === 'space')
      continue
    if (paragraphMedia(block) === null)
      break
    start = index
  }
  if (mediaFrom(blocks, start).media.length !== 1)
    return text
  const before = blocks.slice(0, start).map((block) => block.raw).join('')
  // The lexer reads a carriage return as a line feed; such text is shown as it is.
  return settled.startsWith(before) ? before : text
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
