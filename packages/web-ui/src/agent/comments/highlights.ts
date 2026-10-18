/**
 * The page's comment marks, painted through the CSS Custom Highlight API
 * (`base.css`): the page has one highlight of each kind, so every list that
 * marks comments, as the gallery's specimens do side by side, publishes its
 * ranges here and they are painted together.
 */
export type MarkKind = 'comment' | 'comment-sent' | 'comment-active' | 'comment-selection'

const KINDS: readonly MarkKind[] = ['comment-selection', 'comment-sent', 'comment', 'comment-active']

const published = new Map<symbol, Record<MarkKind, Range[]>>()

/** Publishes the ranges of `owner`, or takes them back with null. */
export function publishMarks(owner: symbol, ranges: Record<MarkKind, Range[]> | null): void {
  if (ranges)
    published.set(owner, ranges)
  else
    published.delete(owner)
  // A browser without the API paints no marks; the comments still work.
  if (typeof Highlight === 'undefined' || !('highlights' in CSS))
    return
  KINDS.forEach((kind, priority) => {
    const highlight = new Highlight(...[...published.values()].flatMap((each) => each[kind]))
    highlight.priority = priority
    CSS.highlights.set(kind, highlight)
  })
}
