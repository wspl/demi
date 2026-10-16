import { lazyChunk } from '../infra/lazy'

// The terminal (`XtermView`, xterm.js and its stylesheet) is a chunk of its
// own, which the first terminal that opens loads: a page that opens none
// downloads none of it. Nothing imports `XtermView.vue` statically.

const xtermChunk = lazyChunk(() => import('./XtermView.vue'))

/** The terminal for `TerminalView`, its load started as the view is set up; `retry` is a failed load's Retry, which loads the page again. */
export const useXterm = xtermChunk.use
