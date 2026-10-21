import { loadEditor } from '../editor/load'
import { ensureFileIconTheme } from '../files/file-icon-theme'
import { loadHighlighterWithCommonLanguages } from '../markdown/highlight'
import { loadKatex } from '../markdown/math'

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
// A region that shows while what it needs loads waits for it (`region-wait.ts`).
//
// Every page that shows a conversation calls this once it has shown: the app,
// a shared conversation and the gallery.

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
      // Each reports its own failure, as it does when a region asks for it.
      void loadHighlighterWithCommonLanguages()
      void loadKatex()
      void loadEditor().catch(() => {})
      void ensureFileIconTheme().catch(() => {})
    })
  })
}
