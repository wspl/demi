import type { ToolMediaSource, ToolResultContentBlock } from '@demicodes/protocol'

/**
 * One thing a call shows under its row (`file-previews.md` § Media a tool
 * returned): an image or a video its result carries, or the line that says a
 * medium is gone and why.
 */
export type ToolMedium =
  | { kind: 'image' | 'video'; source: ToolMediaSource }
  | { kind: 'gone'; text: string }

/** A part of a result that says its image or video is gone. */
type GoneMedium = Extract<ToolResultContentBlock, { type: 'gone' }>

/**
 * The line a medium that is gone shows in its place: that it was not stored,
 * with the store's reason, such as
 * `Video not stored: the object store refused the write`.
 */
export function goneLine(part: GoneMedium): string {
  const kind = part.kind === 'image' ? 'Image' : 'Video'
  return `${kind} not stored: ${part.cause.error}`
}

/** What a call shows under its row, in the order of its result; its text is the call's own. */
export function toolMedia(output: readonly ToolResultContentBlock[]): ToolMedium[] {
  return output.flatMap((part): ToolMedium[] => {
    switch (part.type) {
      case 'text':
        return []
      case 'gone':
        return [{ kind: 'gone', text: goneLine(part) }]
      default:
        return [{ kind: part.type, source: part.source }]
    }
  })
}
