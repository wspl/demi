import { loadEditor } from '../editor/load'
import { ensureFileIconTheme } from '../files/file-icon-theme'
import { loadHighlighterWithCommonLanguages } from '../markdown/highlight'
import { loadKatex } from '../markdown/math'
import { regionsSettled } from './region-wait'

// The one owner of loading what regions need
// (`docs/decisions/0033-a-region-draws-once.md`). Each renderer has one loader,
// idempotent, which a region that needs its renderer also calls on its own
// when it is shown before this runs; this runs them all once the first
// conversation has shown:
//
// - the highlighter with its common languages (`markdown/highlight.ts`)
// - KaTeX and its fonts (`markdown/math.ts`)
// - the code editor (`editor/load.ts`)
// - the file icons' list (`files/file-icon-theme.ts`), not each icon's picture,
//   which loads when it is first shown
//
// A region that shows while what it needs loads waits for it (`region-wait.ts`),
// and these loads wait until no region waits for anything, so that they do not
// take the network from what a region shown waits for, which on a slow link
// would draw it later than it draws without them. A region is a whole chain,
// such as the highlighter and then a language, so the loads do not start
// between its links.
//
// Every page that shows a conversation calls this once it has shown: the app,
// a shared conversation and the gallery.

/** The longest the background load waits for the regions that wait, such as one whose renderer never arrives. */
const MOST_TO_WAIT_MS = 15_000

let started = false

/**
 * Loads, in the background, everything regions need, once. The caller calls it
 * only after the first conversation has shown, so none of it competes with the
 * conversation for the network or the main thread: it starts after the next
 * frame has painted.
 */
export function loadRegionRenderers(): void {
  if (started)
    return
  started = true
  requestAnimationFrame(() => {
    setTimeout(() => {
      void regionsSettled(MOST_TO_WAIT_MS).then(() => {
        // Each reports its own failure, as it does when a region asks for it.
        void loadHighlighterWithCommonLanguages()
        void loadKatex()
        void loadEditor().catch(() => {})
        void ensureFileIconTheme().catch(() => {})
      })
    })
  })
}
