import { Renderer, type MarkedExtension } from 'marked'

/**
 * A table in a scroller of its own (`.table-scroll`), which scrolls sideways
 * a table wider than the text. A message and a document render tables the
 * same way.
 */
export const scrollingTable: MarkedExtension = {
  renderer: {
    table(token) {
      return `<div class="table-scroll">${Renderer.prototype.table.call(this, token)}</div>\n`
    },
  },
}
