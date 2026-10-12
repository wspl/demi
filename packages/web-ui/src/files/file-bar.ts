import { baseName, pluralize } from '@demicodes/utils'
import { languageName } from '../editor/language'
import { entryKind, fileExtension, formatBytes } from './format'
import { formatZoom } from './image-zoom'
import { hasSourceView, previewKind } from './preview'
import type { PixelSize } from './thumbnail'

/** What a media preview knows of the file it shows: its pixel size once loaded, an image's zoom, its size. */
export interface MediaFacts {
  pixels: PixelSize | null
  zoom: number | null
  size: number
}

/** What the File view shows of a file, which its bar names and acts on. */
export type ShownFile =
  /** On its way, or a read that failed: the bar names its kind alone. */
  | { as: 'reading' }
  /** Nothing is at the path. */
  | { as: 'missing' }
  /** Its whole text, as source or rendered. */
  | { as: 'text'; text: string }
  /** The first part of a text file too large to read whole, `size` its whole size. */
  | { as: 'start'; size: number }
  | { as: 'media'; facts: MediaFacts }
  /** Its card, with its size once described. */
  | { as: 'card'; size: number | null }

/** What the File view's bar says and offers for a file (`file-previews.md` § The File view's bars). */
export interface FileBar {
  /** On its left: the kind's full name and its facts, *TypeScript · 19 lines · 476 B*. */
  label: string
  /** Preview and Source. */
  modes: boolean
  /** What Copy puts on the clipboard; null without Copy. */
  copy: string | null
  download: boolean
}

/** A media preview's facts, *1280 × 720 · 150% · 412 KB*: what a picture's caption says and its bar after its kind. */
export function mediaFactParts(facts: MediaFacts): string[] {
  const parts = [
    facts.pixels && facts.pixels.width > 0 ? `${facts.pixels.width} × ${facts.pixels.height}` : null,
    facts.zoom === null ? null : formatZoom(facts.zoom),
    formatBytes(facts.size),
  ]
  return parts.filter((part) => part !== null)
}

/**
 * Lines as GitHub counts them: the line break that ends the last line
 * starts no line of its own, so `a⏎b⏎` and `a⏎b` have 2 and an empty file 0.
 */
function lineCount(text: string): number {
  return text === '' ? 0 : text.replace(/(\r\n?|\n)$/, '').split(/\r\n?|\n/).length
}

const MEDIA_NOUNS = { image: 'Image', video: 'Video', audio: 'Audio', pdf: 'Document' } as const

/**
 * What a file is, in full: *PNG Image*, *TypeScript*, *Plain Text* for text
 * in no language, a card's kind. An SVG shown as its source is text.
 */
function kindName(path: string, shown: ShownFile): string {
  const name = baseName(path)
  const kind = previewKind(path)
  const asText = shown.as === 'text' || shown.as === 'start'
  if (kind !== 'text' && kind !== 'markdown' && !asText)
    return `${fileExtension(name) ?? ''} ${MEDIA_NOUNS[kind]}`.trim()
  if (shown.as === 'card')
    return entryKind(name, false)
  return languageName(path) ?? 'Plain Text'
}

/** The facts after a file's kind. */
function facts(shown: ShownFile): string[] {
  switch (shown.as) {
    case 'text': {
      const lines = lineCount(shown.text)
      return [`${lines.toLocaleString('en-US')} ${pluralize(lines, 'line')}`, formatBytes(new TextEncoder().encode(shown.text).length)]
    }
    // Its lines are not known: the page holds only the start.
    case 'start':
      return [formatBytes(shown.size)]
    case 'media':
      return mediaFactParts(shown.facts)
    case 'card':
      return shown.size === null ? [] : [formatBytes(shown.size)]
    default:
      return []
  }
}

/**
 * The File view's bar for the file at `path` as the view shows it: a file
 * that is not there names itself and offers nothing; any other its kind and
 * facts, Preview and Source for Markdown and SVG, Copy of its whole text
 * while the page holds all of it, and Download.
 */
export function fileBar(path: string, shown: ShownFile): FileBar {
  if (shown.as === 'missing')
    return { label: baseName(path), modes: false, copy: null, download: false }
  return {
    label: [kindName(path, shown), ...facts(shown)].join(' · '),
    modes: hasSourceView(path),
    copy: shown.as === 'text' ? shown.text : null,
    download: true,
  }
}
