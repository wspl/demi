import type { ToolMediaSource, ToolResultContentBlock } from '@demicodes/protocol'
import { toolCallTitle } from './block-helpers'
import type { WorkStep } from './work-groups'

/**
 * One thing a call shows under its row (`file-previews.md` § Media a tool
 * returned): an image or a video its result carries, named by the call's
 * title, or the line that says a medium is gone and why.
 */
export type ToolMedium =
  | { kind: 'image' | 'video'; source: ToolMediaSource; name: string }
  | { kind: 'gone'; text: string }

/** A part of a result that says its medium is gone. */
type GoneMedium = Extract<ToolResultContentBlock, { type: 'gone' }>

/**
 * The line a medium that is gone shows in its place: that it was not stored,
 * with the store's reason, such as
 * `Video not stored: the object store refused the write`.
 */
export function goneLine(part: GoneMedium): string {
  const kind = { image: 'Image', video: 'Video', document: 'Document' }[part.kind]
  return `${kind} not stored: ${part.cause.error}`
}

/**
 * What a call titled `name` shows under its row, in the order of its result;
 * its text is the call's own, and a document shows only as its line in the
 * output.
 */
export function toolMedia(output: readonly ToolResultContentBlock[], name: string): ToolMedium[] {
  return output.flatMap((part): ToolMedium[] => {
    switch (part.type) {
      case 'text':
      case 'document':
        return []
      case 'gone':
        return [{ kind: 'gone', text: goneLine(part) }]
      default:
        return [{ kind: part.type, source: part.source, name }]
    }
  })
}

/**
 * What a folded work group shows under its row: the media of its calls, in
 * the order of the calls, each as it shows under its own call's row; open,
 * each call shows its own (`file-previews.md` § Media a tool returned).
 */
export function foldedMedia(steps: readonly WorkStep[]): ToolMedium[] {
  return steps.flatMap((step) => step.type === 'tool_call' ? toolMedia(step.output, toolCallTitle(step)) : [])
}
