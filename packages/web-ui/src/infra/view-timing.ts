import { pageLog } from './log'

/**
 * How long a view of an agent's transcript took to show its first rows, for the
 * log: `view.opened` starts the clock (`viewOpened`), what supplies rows notes
 * itself on the way (`viewRead`, `viewSupplied`), and the list that first has
 * rows ends it (`viewReady`) with one `view.ready` line.
 *
 * ```text
 *   viewOpened --> viewRead | viewSupplied (offline copy)* --> viewReady
 * ```
 *
 * What the page held already when the view opened has no supplier: its source
 * is `held`.
 *
 * A view is a conversation's root (`node` null) or one of its agents. A view that
 * was never opened, such as a list drawn again by a layout change, has no line.
 */

interface Opening {
  /** The conversation the view is of. */
  conversation: string
  at: number
  /** The reads made for the view so far. */
  reads: number
  /** What supplied the first rows, as noted before they showed; none when the page held them already. */
  source: 'offline_copy' | 'read' | null
}

/** By the agent's ID: the conversation's own agent, its root, has the conversation's ID. */
const openings = new Map<string, Opening>()

/** A view of `node` in `conversation` opens: writes `view.opened` and starts its clock. */
export function viewOpened(conversation: string, node: string | null, from: string): void {
  openings.set(node ?? conversation, { conversation, at: performance.now(), reads: 0, source: null })
  pageLog.info('view.opened', { view: node === null ? 'conversation' : 'subagent', ...(node === null ? {} : { node }), from }, conversation)
}

/** The page read a page of the view's transcript, which counts toward filling its first screen. */
export function viewRead(conversation: string, node: string | null): void {
  const opening = openings.get(node ?? conversation)
  if (opening) {
    opening.reads += 1
    opening.source ??= 'read'
  }
}

/** Something other than a read supplied the view's rows: the offline copy. */
export function viewSupplied(conversation: string, node: string | null, source: 'offline_copy'): void {
  const opening = openings.get(node ?? conversation)
  if (opening) {
    opening.source ??= source
  }
}

/** The view shows its first `rows` rows: writes `view.ready` once for the opening, and ends it. */
export function viewReady(view: string, rows: number): void {
  const opening = openings.get(view)
  if (!opening) {
    return
  }
  openings.delete(view)
  pageLog.info('view.ready', {
    view: opening.conversation === view ? 'conversation' : 'subagent',
    ...(opening.conversation === view ? {} : { node: view }),
    ms_since_opened: Math.round(performance.now() - opening.at),
    source: opening.source ?? 'held',
    rows,
    reads: opening.reads,
  }, opening.conversation)
}

/** The view closed before it showed rows: its opening is dropped, with a line that says it never became ready. */
export function viewAbandoned(conversation: string, node: string | null): void {
  const opening = openings.get(node ?? conversation)
  if (!opening) {
    return
  }
  openings.delete(node ?? conversation)
  pageLog.info('view.abandoned', {
    view: node === null ? 'conversation' : 'subagent',
    ...(node === null ? {} : { node }),
    ms_since_opened: Math.round(performance.now() - opening.at),
    reads: opening.reads,
  }, conversation)
}
