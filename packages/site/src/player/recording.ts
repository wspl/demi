// A demo's recording: the demo app's page as a first snapshot, then each
// change to it at its time, so a frame can show the demo with no app at all.
// `record.ts` makes one for each feature on each screen, by playing the real
// demo (`src/demo`) in a browser; `main.ts` plays it back.

/** A node of the page: a text, a comment, or an element with its attributes and children. */
export type RecordedNode =
  | [id: number, kind: 0, text: string]
  | [id: number, kind: 1]
  | [id: number, tag: string, attributes: Record<string, string>, children: RecordedNode[]]

/** A child in a parent's new list: a node the page already has, by its id, or a new one. */
export type RecordedChild = number | RecordedNode

/** A text range, by its ends' nodes and offsets. */
export type RecordedRange = [startNode: number, startOffset: number, endNode: number, endOffset: number]

/** One change, at its time in milliseconds from the start. */
export type RecordedEvent =
  /** The parent's children are now these. */
  | [ms: number, kind: 'children', parent: number, children: RecordedChild[]]
  /** An attribute changed; null removed it. */
  | [ms: number, kind: 'attribute', node: number, name: string, value: string | null]
  | [ms: number, kind: 'text', node: number, text: string]
  | [ms: number, kind: 'scroll', node: number, top: number, left: number]
  /** A field's value, which no attribute holds. */
  | [ms: number, kind: 'value', node: number, value: string]
  /** A named CSS highlight's ranges; null removed it. */
  | [ms: number, kind: 'highlight', name: string, ranges: RecordedRange[] | null]
  | [ms: number, kind: 'selection', range: RecordedRange | null]
  /**
   * An animation the page's script started on an element, as `Element.animate`
   * takes it, and how far it had run at `ms`: a script may start one part of
   * the way through, as a reply's text that fades in does across renders.
   */
  | [ms: number, kind: 'animate', node: number, keyframes: Keyframe[] | PropertyIndexedKeyframes, options: number | KeyframeAnimationOptions | undefined, currentMs: number | null]
  /**
   * The page's script cancelled an animation, such as a row's glide that a
   * later one replaces: the recording's `index`-th `animate`, from 0.
   */
  | [ms: number, kind: 'cancel', animation: number]
  /** A message the demo sent the site's page. */
  | [ms: number, kind: 'message', data: unknown]

export interface Recording {
  /** The page's root element, its head and its body, as they were when the recording started. */
  root: [id: number, attributes: Record<string, string>]
  head: RecordedNode
  body: RecordedNode
  /** When the demo had shown its first screen: playing starts here. */
  startMs: number
  events: RecordedEvent[]
}
