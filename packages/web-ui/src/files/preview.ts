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

/**
 * Ends what an image or a player still loads when it leaves the page:
 * removing the element leaves its fetch running, and a player keeps
 * downloading, so the element drops its source, and a player loads nothing.
 * A player stops at once. The source goes only once the element has left
 * the document: a component unmounts when its leave transition starts, as a
 * dialog's content does when the dialog starts to fade out, and an element
 * without its source would show a broken image in the picture's place until
 * the fade ends.
 */
export function dropSource(element: HTMLImageElement | HTMLMediaElement): void {
  if (element instanceof HTMLMediaElement)
    element.pause()
  const drop = (): void => {
    element.removeAttribute('src')
    if (element instanceof HTMLMediaElement)
      element.load()
  }
  if (!element.isConnected) {
    drop()
    return
  }
  const left = new MutationObserver(() => {
    if (element.isConnected)
      return
    left.disconnect()
    drop()
  })
  left.observe(element.ownerDocument, { childList: true, subtree: true })
}
