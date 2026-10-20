import type { RendererObject, Token, Tokens } from 'marked'
import { markdownReader } from './reader'
import { baseName } from '@demicodes/utils'
import type { AttachmentLookup, MarkdownRenderOptions, MessageAttachment, MessageFileTarget, MessageFiles } from './types'
import type { HostImage } from '../files/host-image'
import { codeBlockHtml } from './highlight'
import { WEB_LINK_REL, commitFolder, isHttpUrl, linkedCommit, messageFileTarget, messageImage } from './filePath'
import { attachmentBlockAt, attachmentBlockStart } from './attachment-block'
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
import { t } from '../locales/text'

// Parsing is synchronous, so the renderers read the options of the parse in
// flight instead of building a Marked instance (and re-registering KaTeX) per
// call.
let activeOptions: MarkdownRenderOptions | undefined
/**
 * Links open around the renderer: an image inside one follows it instead of
 * opening itself.
 */
let openLinks = 0

/**
 * A link that leaves for the web, in a new tab, with `attributes` after its
 * own.
 */
function webLink(url: string, attributes = ''): string {
  return `<a href="${escapeHtml(url)}" target="_blank" rel="${WEB_LINK_REL}"${attributes}>`
}

/**
 * A link that opens a Host file through the `file` intent, at its line when it
 * names one.
 */
function fileLink({ path, line }: MessageFileTarget, attributes = ''): string {
  const at = line === null ? '' : ` data-line="${line}"`
  return `<a href="${escapeHtml(path)}" data-file-link${at}${attributes}>`
}

/**
 * A link that opens a commit's changes, in the repository of the folder `root`
 * when the link names one.
 */
function commitLink(commit: string, root: string | null, href: string, attributes = ''): string {
  const folder = root === null ? '' : ` data-commit-root="${escapeHtml(root)}"`
  return `<a href="${escapeHtml(href)}" data-commit-link="${escapeHtml(commit)}"${folder}${attributes}>`
}

/**
 * The place of the icon a message's link wears before its text, which the
 * page fills (`StreamedMarkdown`): a Host file's icon
 * (`data-file-icon`, by the file's name), the commit icon (`data-commit-icon`)
 * or a web page's link icon (`data-link-icon`).
 */
function linkIcon(attribute: string): string {
  return `<span class="message-link-icon" ${attribute} aria-hidden="true"></span>`
}

/** The icon of a Host file named `name`. */
function fileIcon(name: string): string {
  return linkIcon(`data-file-icon="${escapeHtml(name)}"`)
}

/**
 * A message's link that wears an icon, every link of a message but one whose
 * text is an image: the link `open` starts, with the classes that space it from
 * the words around it (`icon-link`), then its icon and text (`iconLead`).
 */
function iconLink(open: (attributes: string) => string, icon: string, body: string, tight: boolean): string {
  return `${open(` class="${iconLinkClass(tight)}"`)}${iconLead(icon, body)}</a>`
}

/**
 * The icon or mark of a link and its text. The icon stays with the first
 * word of the text on its line. The icon already says what the link is, so
 * code in its text drops code's look and reads as the words of a link
 * without code do: `` [`runner.md`](runner.md) `` as `[runner.md](runner.md)`.
 * The renderer writes inline code as a bare `<code>` around escaped text, so
 * its tags are the only ones that match.
 */
function iconLead(icon: string, body: string): string {
  const text = body.replace(/<\/?code>/g, '')
  const lead = firstWord(text.slice(0, text.search(/<|$/)))
  return `<span class="icon-lead">${icon}${lead}</span>${text.slice(lead.length)}`
}

/**
 * The classes of a link that starts with an icon, `tight` when ASCII
 * punctuation follows it, which keeps no space after its name.
 */
function iconLinkClass(tight: boolean): string {
  return tight ? 'icon-link icon-link-tight' : 'icon-link'
}

/**
 * Punctuation that stands right after the word before it in Latin text, which a
 * link's own space must not part from it.
 */
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
    // Inline code may become a file's link (`codespan`), so it is marked as a
    // link is.
    if ((part.type === 'link' || part.type === 'codespan') && next?.type === 'text' && CLOSING_PUNCTUATION_RE.test(next.raw))
      tightLinks.add(part)
  })
  // A walk gives each token, but a table's cells hold their inline tokens apart
  // from it.
  agentMarked.walkTokens(tokens, (token) => {
    if (token.type === 'table') {
      for (const cell of [...token.header, ...token.rows.flat()])
        mark(cell.tokens)
    } else if ('tokens' in token && Array.isArray(token.tokens)) {
      mark(token.tokens)
    }
  })
}

/** The links and code spans `tightenLinks` marked, by token. */
const tightLinks = new WeakSet<Token>()

/**
 * A link to agent `number` of the conversation, around its text: the agent's
 * mark, which the page puts in its place with the agent's live state, then the
 * text. A click opens it in the Agents window. A number the conversation does
 * not have shows the text alone; the page's tip says so. Outside a
 * conversation, the text alone.
 *
 * It starts with an icon, so it is spaced as a file's link is (`icon-link`). It
 * is a control of the Agents window, as the window's chip is: a press on it is
 * not one outside the window (`KEEPS_WINDOWS`), so a click when the window
 * shows its tab closes the window.
 */
function agentLink(number: number, body: string, tight: boolean): string {
  const agents = activeOptions?.agents
  if (!agents)
    return body
  if (!agents.find(number))
    return `<span class="agent-link-missing" data-agent-link="${number}">${body}</span>`
  // A button element lays out as one block, so a long name would move whole
  // to a line of its own: the link is inline text that acts as a button.
  return `<span role="button" tabindex="0" class="agent-link ${iconLinkClass(tight)}" data-agent-link="${number}" ${KEEPS_WINDOWS}>`
    + `${iconLead(`<span class="agent-link-mark" data-agent-mark="${number}"></span>`, body)}</span>`
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
 * The attribute of an attachment's image or video that holds the file as Demi's
 * own form of a copy holds it, which a drag of the link into a composer carries
 * (`copied-message.ts` § `attachmentDrag`). /
 */
export const ATTACHMENT_FILE_ATTRIBUTE = 'data-attachment-file'

/**
 * The attribute of a Host image that names its file's path, which a click on it
 * shows in the viewer (`openMessageLink`).
 */
export const HOST_FILE_ATTRIBUTE = 'data-host-file'

/** An attachment as Demi's own form of a copy holds a stored file. */
function attachmentFile(attachment: MessageAttachment): CopiedStoredFile {
  return { type: 'stored', name: attachment.name, sha256: attachment.sha256, mediaType: attachment.mediaType }
}

/**
 * The link around an attachment's image or video that opens it large in the
 * page's viewer, where a video plays. It is the viewer's control: a press on
 * it does not close the viewer, which shows the medium in place of the one
 * it shows (`KEEPS_WINDOWS`).
 */
function attachmentLink(attachment: MessageAttachment, attributes = ''): string {
  const url = escapeHtml(attachment.url)
  const name = escapeHtml(attachment.name)
  const file = ` ${ATTACHMENT_FILE_ATTRIBUTE}="${escapeHtml(JSON.stringify(attachmentFile(attachment)))}"`
  const kind = attachment.mediaType.startsWith('video/') ? 'video' : 'image'
  return `<a href="${url}" data-attachment-${kind}="${name}"${file} ${KEEPS_WINDOWS}${attributes}>`
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
 * a click opens in the viewer to play. It is a tile, as an image is.
 */
function attachmentVideo(attachment: MessageAttachment): string {
  const video = `<video src="${escapeHtml(attachment.url)}" muted playsinline preload="metadata" aria-label="${escapeHtml(attachment.name)}"${sizeAttributes(declaredSize(attachment))}></video>`
  const mark = '<span class="media-play-mark" aria-hidden="true"></span>'
  return `${attachmentLink(attachment, ' class="message-video"')}${video}${mark}</a>`
}

/**
 * The attribute of an audio attachment's place, which holds its address, where
 * `AudioPlayer` plays it.
 */
export const ATTACHMENT_AUDIO_ATTRIBUTE = 'data-attachment-audio'

/**
 * An audio attachment as the place of its inline player, which plays it
 * where it is, as a voice message plays in Messages; `AudioPlayer` fills it
 * once the render is in the page.
 */
function attachmentAudio(attachment: MessageAttachment): string {
  return `<span class="message-audio" ${ATTACHMENT_AUDIO_ATTRIBUTE}="${escapeHtml(attachment.url)}" data-audio-name="${escapeHtml(attachment.name)}"></span>`
}

/**
 * The attribute of the place of an attachment block's card, which holds the
 * attachment's number, where `AttachmentCard` shows it.
 */
export const ATTACHMENT_CARD_ATTRIBUTE = 'data-attachment-card'

/**
 * The attachment `id` a message names, as `files` show it to the agent that
 * wrote the message; none while the page cannot ask.
 */
export function messageAttachment(files: MessageFiles | undefined, id: string): AttachmentLookup | undefined {
  return files?.attachment?.(id, files.node ?? null)
}

/**
 * An attachment block's attachment when it is an image or a video, which stands
 * as a tile among the message's media.
 */
function blockMedium(id: string): MessageAttachment | null {
  const lookup = messageAttachment(activeOptions?.files, id)
  return lookup?.state === 'found' && /^(?:image|video)\//.test(lookup.attachment.mediaType) ? lookup.attachment : null
}

/**
 * An attachment's image or video as a tile, its record's size its box before
 * its bytes arrive, which a click opens large.
 */
function attachmentTile(attachment: MessageAttachment): string {
  if (attachment.mediaType.startsWith('video/'))
    return attachmentVideo(attachment)
  const image = `<img src="${escapeHtml(attachment.url)}" alt="${escapeHtml(attachment.name)}"${sizeAttributes(declaredSize(attachment))} />`
  return `${attachmentLink(attachment)}${image}</a>`
}

/**
 * An attachment block, `raw` the line that names attachment `id`: an image or a
 * video as a lone tile, audio as its inline player, any other file, and one the
 * page still asks for, as the place of its card; a number the conversation does
 * not have says so in the block's place. While the page cannot ask, or asking
 * failed, the line shows as its text.
 */
function attachmentBlock(id: string, raw: string): string {
  const lookup = messageAttachment(activeOptions?.files, id)
  switch (lookup?.state) {
    case 'found': {
      const { attachment } = lookup
      if (/^(?:image|video)\//.test(attachment.mediaType))
        return `<p class="media-lone">${attachmentTile(attachment)}</p>\n`
      if (attachment.mediaType.startsWith('audio/'))
        return `<p>${attachmentAudio(attachment)}</p>\n`
      return `<div class="attachment-block" ${ATTACHMENT_CARD_ATTRIBUTE}="${id}"></div>\n`
    }
    case 'loading':
      return `<div class="attachment-block" ${ATTACHMENT_CARD_ATTRIBUTE}="${id}"></div>\n`
    case 'missing':
      return `<p class="attachment-missing">${escapeHtml(t('markdown.attachmentMissing', { id }))}</p>\n`
    default:
      return `<p>${escapeHtml(raw.trim())}</p>\n`
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
 * An agent's message: read-only task boxes, HTML shown as text, highlighted
 * code, and links and images that reach the web or the conversation's Host.
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
  // Inline code that names a file the Host has opens it, as a link of words
  // with the file's icon.
  codespan(token) {
    const { text } = token
    const code = `<code>${escapeHtml(text)}</code>`
    const files = activeOptions?.files
    const existing = activeOptions?.codeFiles
    if (!files?.open || !existing || openLinks > 0)
      return code
    const target = codeFileTarget(text, files.cwd)
    return target && existing.has(target.path) ? iconLink((attributes) => fileLink(target, attributes), fileIcon(baseName(target.path)), code, tightLinks.has(token)) : code
  },
  link(token) {
    openLinks += 1
    let body: string
    try {
      body = this.parser.parseInline(token.tokens)
    } finally {
      openLinks -= 1
    }
    // A link whose text is a picture, as a badge's is, shows the picture alone.
    const named = !(token.tokens ?? []).some((part) => part.type === 'image')
    const tight = tightLinks.has(token)
    const link = (open: (attributes: string) => string, icon: string) =>
      named ? iconLink(open, icon, body, tight) : `${open('')}${body}</a>`
    if (isHttpUrl(token.href))
      return link((attributes) => webLink(token.href, attributes), linkIcon('data-link-icon'))
    const agent = linkedAgent(token.href)
    if (agent !== null)
      return agentLink(agent, body, tight)
    // A commit opens its changes; nobody to open it, its text alone.
    const commit = linkedCommit(token.href)
    if (commit !== null) {
      const files = activeOptions?.files
      // A folder is read as a relative link's path is; one that names nothing,
      // such as `~/` before the home is known, is text.
      const root = commit.folder === null || !files ? null : commitFolder(commit.folder, files)
      if (!files?.openCommit || (commit.folder !== null && root === null))
        return body
      return link((attributes) => commitLink(commit.commit, root, token.href, attributes), linkIcon('data-commit-icon'))
    }
    // A file nobody opens is named as text.
    const files = activeOptions?.files
    const target = files?.open ? messageFileTarget(token.href, files) : null
    if (target === null)
      return body
    return link((attributes) => fileLink(target, attributes), fileIcon(baseName(target.path)))
  },
  image(token) {
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
 * Whether an image token shows a picture: one that loads, also while its
 * Host file is read. One that never shows, which leaves its alt text, is
 * words.
 */
function showsMedium(token: Token): boolean {
  return token.type === 'image' && imageSource(token.href) !== null
}

/**
 * Whether an inline token is a medium of the message: an image that shows, or a
 * link around one and nothing else.
 */
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
 * alone, or a run of two or more side by side.
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

/**
 * The words of a paragraph between its media, without the white space and line
 * breaks at their edges.
 */
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

/**
 * Joins media blocks that follow each other, across the blank lines between
 * them, into one run.
 */
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
 * words come before the first of them in its paragraph, an attachment
 * block's image or video among them; any other text, a list or a heading
 * ends a run.
 */
function separateMedia(blocks: Token[]): void {
  for (let index = 0; index < blocks.length; index += 1) {
    const block = blocks[index]
    if (block?.type === 'list') {
      for (const item of block.items)
        separateMedia(item.tokens)
    } else if (block?.type === 'blockquote') {
      separateMedia(block.tokens ?? [])
    } else if (block?.type === ATTACHMENT_BLOCK && blockMedium(block.id) !== null) {
      blocks[index] = mediaBlock([block])
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

/** The token of an attachment block (`attachment-block.ts`). */
const ATTACHMENT_BLOCK = 'attachmentBlock'

const agentMarked = markdownReader({ breaks: true, renderer: messageRenderer }, scrollingTable)
// LaTeX math, rendered to HTML whose stylesheet comes with KaTeX (`math.ts`);
// `throwOnError` keeps malformed math from blowing up the whole message.
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
    // Ahead of an HTML block, which is shown as text, and cutting a
    // paragraph short at its line, so a block on the line after words
    // stands on its own.
    name: ATTACHMENT_BLOCK,
    level: 'block',
    start: attachmentBlockStart,
    tokenizer(src) {
      const block = attachmentBlockAt(src)
      return block ? { type: ATTACHMENT_BLOCK, raw: block.raw, id: block.id } : undefined
    },
    renderer(token) {
      return attachmentBlock(token.id, token.raw)
    },
  }, {
    name: MEDIA,
    // Each medium is a tile (`media-run.ts` sizes it once the message's
    // width is known), lone or side by side with the others of its run.
    renderer(token) {
      const media = token.tokens ?? []
      const block = media.length === 1 ? 'media-lone' : 'media-run'
      const tiles = media.map((medium) => {
        const attachment = medium.type === ATTACHMENT_BLOCK ? blockMedium(medium.id) : null
        return attachment ? attachmentTile(attachment) : this.parser.parseInline([medium])
      })
      return `<p class="${block}">${tiles.join('')}</p>\n`
    },
  }],
})

/**
 * The text of each inline code span of an agent's message, in order, at any
 * depth.
 */
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
 * each image that loads and each attachment block that is an image or a
 * video, named and sized as a click on it names and sizes it. An image
 * inside a link follows the link and is not among them; neither is one still
 * being asked for, or one that shows its alt text.
 */
export function messageMedia(src: string, files: MessageFiles | undefined): ShownMedium[] {
  if (!src.includes('![') && !src.includes('<attachment'))
    return []
  const tokens = agentMarked.lexer(src)
  const linked = new Set<Token>()
  agentMarked.walkTokens(tokens, (token) => {
    if (token.type === 'link')
      agentMarked.walkTokens(token.tokens ?? [], (inside) => void linked.add(inside))
  })
  const media: ShownMedium[] = []
  agentMarked.walkTokens(tokens, (token) => {
    if (token.type === ATTACHMENT_BLOCK) {
      const lookup = messageAttachment(files, token.id)
      if (lookup?.state !== 'found')
        return
      const { attachment } = lookup
      const kind = attachment.mediaType.startsWith('image/') ? 'image' : attachment.mediaType.startsWith('video/') ? 'video' : null
      if (kind)
        media.push({ kind, src: attachment.url, name: attachment.name, size: declaredSize(attachment) })
      return
    }
    if (token.type !== 'image' || linked.has(token))
      return
    const image = messageImage(token.href, files)
    const name = token.text || 'image'
    if (image && 'url' in image)
      media.push({ kind: 'image', src: image.url, name })
    // A Host image is its file, which the viewer shows at the version the page
    // keeps.
    else if (image && files?.contents)
      media.push({ kind: 'image', file: { path: image.path, contents: files.contents }, name })
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
 * arriving after it can change, as offsets into it in order.
 *
 * The blocks are the top-level blocks the message's own reader finds, so a
 * blank line inside a fence, a list item or an indented code block never ends
 * one, and a fence still open runs to the end. They are read from the lines
 * that have ended, since the rest of a line can change what the lines before it
 * are: `1. a`, a blank line and `2` are a list and a paragraph until `. b`
 * makes them one list, and `x | y` above `- | -` reads as a table's head until
 * the line goes on as text.
 *
 * Every block before the last is settled, except:
 *
 * - a block that opens display math before its closing line (`MATH_OPENER`),
 *   and every block after it
 * - a raw HTML block, and every block after it: only the whole text nests what
 *   follows `<details>` inside it
 */
export function messageBlockEnds(src: string): number[] {
  const blocks: { start: number; live: boolean }[] = []
  let offset = 0
  for (const token of new agentMarked.Lexer(agentMarked.defaults).blockTokens(src.slice(0, src.lastIndexOf('\n') + 1))) {
    if (token.type !== 'space')
      blocks.push({ start: offset, live: token.type === 'html' || (token.type !== 'blockMath' && MATH_OPENER.test(token.raw)) })
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
