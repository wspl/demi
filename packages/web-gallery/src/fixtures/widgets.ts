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

/**
 * The facts of a reply block the gallery's replies name, as a transcript
 * brings them with its replies: the widgets' frames and titles.
 */
export function galleryReplyBlock(tag: string, id: string): ReplyBlockLookup {
  const widget = tag === 'widget' ? GALLERY_WIDGETS[id] : undefined
  return widget ? { state: 'found', facts: { frame: widget.frame, title: widget.title } } : { state: 'missing' }
}

/** The widget plugin as the gallery's replies reach it: the guide's pages, by their IDs. */
export const galleryWidgets: GalleryPlugin = {
  call: async (method) => {
    throw new Error(`the widget plugin has no method ${method}`)
  },
  // The gallery's server answers the backend's route (`vite.config.ts`), so
  // the document loads the plugin's files from the same root as it would.
  blockDocument: (tag, id) =>
    GALLERY_WIDGETS[id] ? `/api/plugins/widget/blocks/${tag}/${id}` : 'about:blank',
}

/** The pages whose blocks the gallery's replies draw. */
export const WIDGET_PAGES = [widgetPage]
