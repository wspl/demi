import type { CommandReport, ToolMediaSource, ToolResultContentBlock } from '@demicodes/protocol'
import { toolCallTitle } from './block-helpers'
import { reportShown } from './command-reports'
import { t } from './locales/text'
import type { WorkStep } from './work-groups'

/**
 * One thing a call shows of its result's media (`file-previews.md` § Media
 * a tool returned): an image or a video its result carries, named by the
 * call's title, small in the call's row, or the line that says a medium is
 * gone and why, under it.
 */
export type ToolMedium =
  | { kind: 'image' | 'video'; source: ToolMediaSource; name: string }
  | { kind: 'gone'; text: string }

/** An image or a video a tool returned, which shows small in its row's line. */
export type ToolPicture = Extract<ToolMedium, { kind: 'image' | 'video' }>

/**
 * A row's media apart (`file-previews.md` § Media a tool returned): its
 * pictures, which stand in its line, and the lines of those that are gone,
 * which stand under it.
 */
export function splitMedia(media: readonly ToolMedium[]): { pictures: ToolPicture[]; gone: string[] } {
  return {
    pictures: media.filter((medium): medium is ToolPicture => medium.kind !== 'gone'),
    gone: media.flatMap((medium) => (medium.kind === 'gone' ? [medium.text] : [])),
  }
}

/** A part of a result that says its medium is gone. */
type GoneMedium = Extract<ToolResultContentBlock, { type: 'gone' }>

/**
 * The line a medium that is gone shows in its place: that it was not stored,
 * with the store's reason, such as
 * `Video not stored: the object store refused the write`.
 */
export function goneLine(part: GoneMedium): string {
  return t('transcript.gone.line', { kind: t(`transcript.gone.${part.kind}`), reason: part.cause.error })
}

/**
 * What a call titled `name` shows of its result's media, in their order;
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

/** What a command's end report shows of its media, in its notice row, as a call shows its result's. */
export function reportMedia(report: CommandReport): ToolMedium[] {
  return toolMedia(report.media ?? [], report.title)
}

/**
 * What a folded work group shows of its steps' media: the media of its
 * calls and of its command notices, in their order, each named as its own
 * row names it; open, each row shows its own (`file-previews.md` § Media a
 * tool returned).
 */
export function foldedMedia(steps: readonly WorkStep[]): ToolMedium[] {
  return steps.flatMap((step) => {
    switch (step.type) {
      case 'tool_call':
        return toolMedia(step.output, toolCallTitle(step))
      case 'wakeup':
        return step.reports.filter(reportShown).flatMap(reportMedia)
      default:
        return []
    }
  })
}
