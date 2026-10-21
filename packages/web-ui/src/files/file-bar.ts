import { baseName } from '@demicodes/utils'
import { formatCount } from '../ui/numbers'
import { entryKind, fileExtension, formatBytes } from './format'
import { t } from './locales/text'
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

/**
 * The full name of the language a file's text is colored as, by its path,
 * as the editor names it (`editor/language.ts`), which the editor's chunk
 * brings; null for text in no language.
 */
export type LanguageName = (path: string) => string | null

/** What the File view's bar says and offers for a file. */
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

/** The catalogue's text for a media file's kind, `kind.image`. */
const MEDIA_KINDS = { image: 'kind.image', video: 'kind.video', audio: 'kind.audio', pdf: 'kind.document' } as const

/**
 * What a file is, in full: *PNG Image*, *TypeScript*, *Plain Text* for text
 * in no language, a card's kind. An SVG shown as its source is text.
 */
function kindName(path: string, shown: ShownFile, languageName: LanguageName | null): string | null {
  const name = baseName(path)
  const kind = previewKind(path)
  const asText = shown.as === 'text' || shown.as === 'start'
  if (kind !== 'text' && kind !== 'markdown' && !asText)
    return t(MEDIA_KINDS[kind], { extension: fileExtension(name) ?? '' }).trim()
  if (shown.as === 'card')
    return entryKind(name, false)
  // Until the editor's chunk is there, a text file's language is not known.
  return languageName === null ? null : languageName(path) ?? t('kind.plainText')
}

/** What a file is, in full, by its name alone, as a card names it: *PDF Document*, *ZIP file*. */
export function fileKindName(path: string): string {
  return kindName(path, { as: 'card', size: null }, () => null) ?? ''
}

/** The facts after a file's kind. */
function facts(shown: ShownFile): string[] {
  switch (shown.as) {
    case 'text': {
      const lines = lineCount(shown.text)
      return [t('kind.lines', { count: formatCount(lines) }, lines), formatBytes(new TextEncoder().encode(shown.text).length)]
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
 * The File view's bar for the file at `path` as the view shows it.
 *
 * - A file that is not there names itself and offers nothing.
 * - Any other shows its kind and facts, Preview and Source for Markdown and SVG,
 *   Copy of its whole text while the page holds all of it, and Download.
 * - Without a language name, the editor's chunk not being there yet, a text
 *   file's label holds its facts alone, so the bar stands at its size and the
 *   kind joins it.
 */
export function fileBar(path: string, shown: ShownFile, languageName: LanguageName | null): FileBar {
  if (shown.as === 'missing')
    return { label: baseName(path), modes: false, copy: null, download: false }
  return {
    label: [kindName(path, shown, languageName), ...facts(shown)].filter((part) => part !== null).join(' · '),
    modes: hasSourceView(path),
    copy: shown.as === 'text' ? shown.text : null,
    download: true,
  }
}
