import { previewMediaType } from '@demicodes/protocol'

/** What a view shows a file as (`file-previews.md` § What the user sees). */
export type PreviewKind = 'image' | 'video' | 'audio' | 'pdf' | 'markdown' | 'text'

/** The viewer for a file, by the media type its extension names; anything else is read as text. */
export function previewKind(path: string): PreviewKind {
  const type = previewMediaType(path)
  if (type === null)
    return 'text'
  if (type === 'text/markdown')
    return 'markdown'
  if (type === 'application/pdf')
    return 'pdf'
  const top = type.slice(0, type.indexOf('/'))
  return top === 'image' || top === 'video' || top === 'audio' ? top : 'text'
}

/** Markdown and SVG are text as well: they have a rendered view and their source. */
export function hasSourceView(path: string): boolean {
  const type = previewMediaType(path)
  return type === 'text/markdown' || type === 'image/svg+xml'
}

/** What a card says of a file too large for its source to serve or read, unless its view says more. */
export const TOO_LARGE_NOTE = 'Too large to show.'

/** An SVG's text as an image the page can show: a `data:` URL runs no script in the page, even opened on its own. */
export function svgImageUrl(text: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(text)}`
}
