import { shallowRef, type ShallowRef } from 'vue'

// How a region of the page waits for what draws it
// (`docs/decisions/0033-a-region-draws-once.md`). A region is a code block or a
// run of code, a formula, a file icon or an editor; what it needs may be a
// chain of things, such as the highlighter and then a language, or a list and
// then a picture. A region asks `regionForm` with a key that names it and a
// `ready` that says whether everything it needs is in, and is told which form
// to draw:
//
//   held    nothing yet, at the size it will have where that is known
//   plain   its plain form, which stays until the final one replaces it
//   final   everything is in
//
//   shown ──────── 300 ms ────────▶ plain ──────────────▶ final
//     │ held                                  all in
//     └─────────────────────── all in ───────────────────▶ final
//
// The 300 ms run from the first ask for the region's key, once for the whole
// chain, however many things it waits for, and the form never goes back:
// plain stays until final. A form is read inside a computed or a render, which
// then runs again when it changes.

/** How long a region waits before it draws its plain form. */
export const REGION_WAIT_MS = 300

/** How a region draws now. */
export type RegionForm = 'held' | 'plain' | 'final'

interface Region {
  form: RegionForm
  ready: () => boolean
  version: ShallowRef<number>
  /** Called when the form changes by the clock, for what draws without reading `version`. */
  redraw: (() => void) | undefined
}

const regions = new Map<string, Region>()
/** The form every region is held at, for the gallery's specimens and tests; null for each region's own. */
const pinned = shallowRef<RegionForm | null>(null)
/** The regions not final yet. */
const pending = new Set<Region>()
let settledWaiters: (() => void)[] = []

/**
 * How the region `key` draws: `ready` says whether everything it needs is in,
 * and is read now and when `regionsChanged` says something arrived; the
 * latest to ask for a key supplies it. `redraw`
 * is called when the clock turns it from held to plain, by a region whose
 * draw does not read the form inside a computed.
 */
export function regionForm(key: string, ready: () => boolean, redraw?: () => void): RegionForm {
  if (pinned.value !== null)
    return pinned.value
  let region = regions.get(key)
  if (!region) {
    if (ready())
      return 'final'
    const started: Region = { form: 'held', ready, version: shallowRef(0), redraw }
    region = started
    regions.set(key, started)
    pending.add(started)
    setTimeout(() => {
      if (started.form === 'held') {
        started.form = 'plain'
        started.version.value += 1
        started.redraw?.()
      }
    }, REGION_WAIT_MS)
  }
  void region.version.value
  // The latest region to ask says what is all in, as an earlier one may be gone.
  region.ready = ready
  if (region.form !== 'final' && region.ready()) {
    // The readers that follow the version run again at once; this one reads final now.
    const arrived = region
    arrived.form = 'final'
    pending.delete(arrived)
    queueMicrotask(() => {
      arrived.version.value += 1
      announceIfSettled()
    })
  }
  return region.form
}

/** Tells that something a region may wait for has arrived: every region whose chain is complete is final. */
export function regionsChanged(): void {
  for (const region of [...pending]) {
    if (region.ready()) {
      region.form = 'final'
      pending.delete(region)
      region.version.value += 1
    }
  }
  announceIfSettled()
}

function announceIfSettled(): void {
  if (pending.size > 0)
    return
  const waiting = settledWaiters
  settledWaiters = []
  for (const resume of waiting)
    resume()
}

/**
 * Resolves once no region is waiting for anything, or after `timeoutMs`, such
 * as for one that never arrives. What loads for no region yet waits until
 * then, so that it does not take the network from what a region shown waits
 * for; the caller reads it again, as it starts each load.
 */
export function regionsSettled(timeoutMs: number): Promise<void> {
  if (pending.size === 0)
    return Promise.resolve()
  return new Promise((resolve) => {
    const timer = setTimeout(resume, timeoutMs)
    function resume(): void {
      clearTimeout(timer)
      resolve()
    }
    settledWaiters.push(resume)
  })
}

/**
 * Draws every region in `form`, whatever has arrived, or each in its own with
 * null: the gallery shows a form this way. What draws without reading a form
 * inside a computed draws again by `renderAgain` (`markdown/render-version.ts`).
 */
export function pinRegionForms(form: RegionForm | null): void {
  pinned.value = form
}

/** Whether a region is waiting for something now. */
export function regionsWaiting(): boolean {
  return pending.size > 0
}

/** Forgets every region, for a test that starts from a page with nothing loaded. */
export function resetRegionWaits(): void {
  regions.clear()
  pending.clear()
  settledWaiters = []
}
