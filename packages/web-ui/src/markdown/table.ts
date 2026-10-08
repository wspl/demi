import { Renderer, type MarkedExtension } from 'marked'

/**
 * A table in a scroller of its own (`.table-scroll`): the table keeps its
 * natural width, so a short cell stays on one line, and a table wider than
 * the text scrolls sideways inside the scroller rather than squeezing its
 * columns. A message and a document render tables the same way.
 */
export const scrollingTable: MarkedExtension = {
  renderer: {
    table(token) {
      return `<div class="table-scroll">${Renderer.prototype.table.call(this, token)}</div>\n`
    },
  },
}
