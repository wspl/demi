import { toRaw } from 'vue'
import type { ShownMedium } from '../files/media-viewer'
import { declaredSize, type PixelSize } from '../files/thumbnail'
import { ATTACHMENT_MARK } from '../markdown/user-markdown'
import { messageMediumReferences, referencedMedia, type MessageMediumReference } from '../markdown/render'
import type { ConversationFiles, MessageFiles } from '../markdown/types'
import { previewKind } from '../files/preview'
import { toolCallTitle } from './block-helpers'
import { reportShown } from './command-reports'
import { contentMessage, type MessageCapsule } from './message-editor/capsules'
import type { MediaSource } from './media-source'
import { childBrief } from './subagents'
import { reportMedia, toolMedia, type ToolMedium } from './tool-media'
import type { MessageListBlock } from './pending-steers'

/** What a transcript's pictures are read against. */
export interface TranscriptPicturesContext {
  /**
   * The agent whose transcript it is: null for the conversation's own, a
   * subagent's id otherwise.
   */
  node: string | null
  /**
   * The files a block's messages name, resolved from the directory the agent
   * wrote the block in.
   */
  files(blockId: string): MessageFiles | undefined
  /** Where a medium's bytes load from. */
  address(source: MediaSource): string
}

/**
 * The images and videos a list's `rows` show, in the order they stand, from the
 * oldest to the newest: the images the messages show, the user's attached
 * files, and the media the tools and the command reports returned. It is what
 * the media viewer steps through, and holds only what `rows` holds, each
 * picture marked with its row.
 */
export function transcriptPictures(rows: readonly MessageListBlock[], context: TranscriptPicturesContext): ShownMedium[] {
  return rows.flatMap((row) => (row.type === 'work_group' ? row.steps : [row]).flatMap((block) =>
    blockPictures(block, context).map((medium) => ({ ...medium, row: row.id }))))
}

/**
 * The pictures each block's texts name, by the text, as the block's last
 * read found them. The list reads its pictures again on many changes around
 * them, such as a message's attachment answered or a panel opened, and
 * reading a text lexes it whole, so a text is lexed once for each version of
 * it and a long transcript's unchanged messages never again.
 */
const named = new WeakMap<object, ReadonlyMap<string, readonly MessageMediumReference[]>>()

/** The pictures `texts`, the texts of `block` in order, name: each as kept, or read now. */
function namedIn(block: MessageListBlock, texts: readonly string[]): (readonly MessageMediumReference[])[] {
  const key = toRaw(block)
  const kept = named.get(key)
  const read = texts.map((text) => [text, kept?.get(text) ?? messageMediumReferences(text)] as const)
  named.set(key, new Map(read))
  return read.map(([, references]) => references)
}

/** The pictures one block of a row shows. */
function blockPictures(block: MessageListBlock, context: TranscriptPicturesContext): ShownMedium[] {
  switch (block.type) {
    case 'user':
    case 'steer': {
      const files = context.files(block.id)
      const brief = childBrief(context.node, block.content)
      if (brief !== null)
        return namedIn(block, [brief]).flatMap((references) => referencedMedia(references, files, new Map()))
      const { markdown, capsules } = contentMessage(block.content)
      // Each file stands at a mark between the text around it.
      return namedIn(block, markdown.split(ATTACHMENT_MARK)).flatMap((references, index) => {
        const capsule = capsules[index]
        const medium = capsule ? capsuleMedium(capsule, context.address, files) : null
        return [...referencedMedia(references, files), ...(medium ? [medium] : [])]
      })
    }
    case 'text':
      return namedIn(block, [block.text]).flatMap((references) =>
        referencedMedia(references, context.files(block.id), new Map(block.pictures?.map((picture) => [picture.target, picture.copy]))))
    case 'tool_call':
      return toolPictures(toolMedia(block.output, toolCallTitle(block)), context)
    case 'wakeup':
      return toolPictures(block.reports.filter(reportShown).flatMap(reportMedia), context)
    default:
      return []
  }
}

/** The pixel size a media source's reference carries, when it carries one. */
function sourceSize(source: MediaSource): PixelSize | null {
  return 'width' in source ? declaredSize(source) : null
}

function toolPictures(media: readonly ToolMedium[], context: TranscriptPicturesContext): ShownMedium[] {
  return media.flatMap((medium): ShownMedium[] =>
    medium.kind === 'gone'
      ? []
      : [{ kind: medium.kind, src: context.address(medium.source), name: medium.name, size: sourceSize(medium.source) }])
}

/**
 * The picture or the video a user's attached file shows, as the transcript's
 * series holds it and its capsule opens it; null for a file of another kind.
 *
 * - A file the message holds, such as one sent from the computer, loads from
 *   where `address` says its media source does, with the size in bytes the
 *   message's reference to it carries.
 * - A file of the conversation's Host the message names, such as one Attach
 *   Remote File chose there, is that file, read through `files`, as a Host
 *   image of a message is; its kind comes from its name. One on another
 *   device shows nothing here, since `files` reads only the conversation's
 *   Host.
 */
export function capsuleMedium(
  capsule: MessageCapsule,
  address: (source: MediaSource) => string,
  files: ConversationFiles | undefined,
): ShownMedium | null {
  const { image, video, name, reference, path } = capsule
  const contents = files?.contents
  const onHost = reference?.type === 'device' ? reference.deviceId === files?.device : capsule.host === undefined
  if (image === undefined && video === undefined && path !== undefined && contents && onHost) {
    const kind = previewKind(path)
    return kind === 'image' || kind === 'video' ? { kind, name, file: { path, contents } } : null
  }
  const bytes = reference && 'size' in reference ? reference.size ?? null : null
  if (image !== undefined)
    return { kind: 'image', src: typeof image === 'string' ? image : address(image), name, size: typeof image === 'string' ? null : sourceSize(image), bytes }
  if (video !== undefined)
    return { kind: 'video', src: address(video), name, size: sourceSize(video), bytes }
  return null
}
