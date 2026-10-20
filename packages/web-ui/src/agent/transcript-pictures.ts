import type { ShownMedium } from '../files/media-viewer'
import { declaredSize, type PixelSize } from '../files/thumbnail'
import { ATTACHMENT_MARK } from '../markdown/user-markdown'
import { messageMedia } from '../markdown/render'
import type { MessageFiles } from '../markdown/types'
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

/** The pictures one block of a row shows. */
function blockPictures(block: MessageListBlock, context: TranscriptPicturesContext): ShownMedium[] {
  switch (block.type) {
    case 'user':
    case 'steer': {
      const files = context.files(block.id)
      const brief = childBrief(context.node, block.content)
      if (brief !== null)
        return messageMedia(brief, files)
      const { markdown, capsules } = contentMessage(block.content)
      // Each file stands at a mark between the text around it.
      return markdown.split(ATTACHMENT_MARK).flatMap((text, index) => {
        const capsule = capsules[index]
        return [...messageMedia(text, files), ...(capsule ? capsuleMedia(capsule, context) : [])]
      })
    }
    case 'text':
      return messageMedia(block.text, context.files(block.id))
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
 * The picture or the video a user's attached file shows; none for a file of
 * another kind.
 */
function capsuleMedia(capsule: MessageCapsule, context: TranscriptPicturesContext): ShownMedium[] {
  const { image, video, name } = capsule
  if (image !== undefined)
    return [{ kind: 'image', src: typeof image === 'string' ? image : context.address(image), name, size: typeof image === 'string' ? null : sourceSize(image) }]
  if (video !== undefined)
    return [{ kind: 'video', src: context.address(video), name, size: sourceSize(video) }]
  return []
}
