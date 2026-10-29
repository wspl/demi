import widgetPage from '@demicodes/plugin-widget'
import { GALLERY_WIDGETS } from '../generated/widgets'
import type { ReplyBlockLookup } from '@demicodes/web-ui/markdown/types'
import type { GalleryPlugin } from './plugins'

/** A reply that shows three widgets among its sentences: a chart, a grid and a diagram. */
export const widgetMarkdown = `Orders rose in each week of the quarter, and the gap to last year widened from September:

<widget id="w1" />

Evenings carry the week, weekends most:

<widget id="w2" />

The new checkout folds address and payment into one step:

<widget id="w3" />

The jump in September follows the new checkout.`

/** Each widget's document as a blob, made once, as the backend's route would serve it. */
const documents = new Map<string, string>()

/**
 * The facts of a reply block the gallery's replies name, as a transcript
 * brings them with its replies: the widgets' frames and names.
 */
export function galleryReplyBlock(tag: string, id: string): ReplyBlockLookup {
  const widget = tag === 'widget' ? GALLERY_WIDGETS[id] : undefined
  return widget ? { state: 'found', facts: { frame: widget.frame, name: widget.name } } : { state: 'missing' }
}

/** The widget plugin as the gallery's replies reach it: the guide's pages, by their IDs. */
export const galleryWidgets: GalleryPlugin = {
  call: async (method) => {
    throw new Error(`the widget plugin has no method ${method}`)
  },
  blockDocument: (_tag, id) => {
    const widget = GALLERY_WIDGETS[id]
    if (!widget)
      return 'about:blank'
    let url = documents.get(id)
    if (!url) {
      url = URL.createObjectURL(new Blob([widget.document], { type: 'text/html; charset=utf-8' }))
      documents.set(id, url)
    }
    return url
  },
}

/** The pages whose blocks the gallery's replies draw. */
export const WIDGET_PAGES = [widgetPage]
