import type { ToolMediaSource, ToolResultContentBlock } from '@demicodes/protocol'

/**
 * One thing a call shows under its row (`file-previews.md` § Media a tool
 * returned): an image or a video its result carries, or the text that took
 * a medium's place, as the model reads it.
 */
export type ToolMedium =
  | { kind: 'image' | 'video'; source: ToolMediaSource }
  | { kind: 'gone'; text: string }

/**
 * Whether a part of a result is the text that took a medium's place: a
 * medium that was not stored, or one retired after 30 days. Such a text is
 * a part of its own that starts with `[image` or `[video`, and no other part
 * of a tool result starts so (`runtime.md` § Media). The one place the page
 * decides it, until a medium that is gone has a part of its own.
 */
export function takesMediumPlace(part: ToolResultContentBlock): boolean {
  return part.type === 'text' && (part.text.startsWith('[image') || part.text.startsWith('[video'))
}

/** What a call shows under its row, in the order of its result; its other text is the call's own. */
export function toolMedia(output: readonly ToolResultContentBlock[]): ToolMedium[] {
  return output.flatMap((part): ToolMedium[] => {
    if (part.type !== 'text') {
      return [{ kind: part.type, source: part.source }]
    }
    return takesMediumPlace(part) ? [{ kind: 'gone', text: part.text }] : []
  })
}
