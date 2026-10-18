/**
 * The page's comment and side question marks, painted through the CSS Custom Highlight API
 * (`base.css`): the page has one highlight of each kind, so every list that
 * marks comments, as the gallery's specimens do side by side, publishes its
 * ranges here and they are painted together.
 */
export type MarkKind = 'comment' | 'comment-sent' | 'comment-active' | 'comment-selection' | 'ask' | 'ask-active'

/** In the order they paint, the last over the others. */
const KINDS: readonly MarkKind[] = ['comment-selection', 'comment-sent', 'comment', 'ask', 'ask-active', 'comment-active']

const published = new Map<symbol, Record<MarkKind, StaticRange[]>>()

/**
 * Publishes the ranges of `owner`, or takes them back with null. They are
 * painted as static copies: the page moves a list as a whole, as a window
 * opening beside the transcript moves its ancestors, which collapses every
 * live Range inside it though its words are unchanged and nothing the
 * owner watches changed (`anchors.ts`, `liveOf`).
 */
export function publishMarks(owner: symbol, ranges: Record<MarkKind, readonly Range[]> | null): void {
  // A browser without the API paints no marks; the comments still work.
  if (typeof Highlight === 'undefined' || !('highlights' in CSS))
    return
  if (ranges) {
    const copy = (kind: MarkKind) => ranges[kind].map((range) => new StaticRange(range))
    published.set(owner, {
      'comment-selection': copy('comment-selection'),
      'comment-sent': copy('comment-sent'),
      'comment': copy('comment'),
      'ask': copy('ask'),
      'ask-active': copy('ask-active'),
      'comment-active': copy('comment-active'),
    })
  } else
    published.delete(owner)
  KINDS.forEach((kind, priority) => {
    const highlight = new Highlight(...[...published.values()].flatMap((each) => each[kind]))
    highlight.priority = priority
    CSS.highlights.set(kind, highlight)
  })
}
