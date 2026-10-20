import { StateEffect, StateField, type Extension } from '@codemirror/state'
import { Decoration, EditorView, type DecorationSet } from '@codemirror/view'
import { revealOffset } from '../ui/reveal'

/** Marks the line that starts at a position, or ends the mark (null). */
const markLine = StateEffect.define<number | null>()

/** The line a link opened the file at, marked as a search result is (`found-highlight`), until its flash has played. */
const foundLine = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, transaction) {
    let next = marks.map(transaction.changes)
    for (const effect of transaction.effects) {
      if (effect.is(markLine))
        next = effect.value === null ? Decoration.none : Decoration.set([Decoration.line({ class: 'found-highlight' }).range(effect.value)])
    }
    return next
  },
  provide: (field) => EditorView.decorations.from(field),
})

/**
 * Scrolls the code view alone to the line a link opened, and nothing around it.
 * CodeMirror's own scrolling also moves every scrolling ancestor, up to the page,
 * so that the line stands at the top of each. Other scrolling, such as the caret's
 * while typing, keeps CodeMirror's.
 */
const revealInView = EditorView.scrollHandler.of((view, range, { y, yMargin }) => {
  const marked = view.state.field(foundLine)
  let revealing = false
  marked.between(range.head, range.head, (from) => {
    revealing ||= from === range.head
  })
  if (!revealing || y !== 'start')
    return false
  const scroller = view.scrollDOM
  const lineTop = view.documentTop + view.lineBlockAt(range.head).top
  scroller.scrollTop += lineTop - scroller.getBoundingClientRect().top - yMargin
  // The line opens at its start, at the left edge.
  scroller.scrollLeft = 0
  return true
})

/**
 * What lets a code view open at a line (`revealLine`): the mark goes once
 * its flash has played, so a line drawn again later does not flash again.
 */
export function foundLineExtension(): Extension {
  return [
    foundLine,
    revealInView,
    EditorView.domEventHandlers({
      animationend(event, view) {
        if (event.animationName === 'found-highlight' && view.state.field(foundLine).size > 0)
          view.dispatch({ effects: markLine.of(null) })
      },
    }),
  ]
}

/**
 * Brings line `line` (1-based, held to the text's lines) into view as every
 * reveal places its block (`revealOffset`), puts the cursor at its start,
 * and marks it for a moment, as a search result's block is.
 */
export function revealLine(view: EditorView, line: number): void {
  const doc = view.state.doc
  const at = doc.line(Math.min(Math.max(line, 1), doc.lines))
  const yMargin = revealOffset(view.lineBlockAt(at.from).height, view.scrollDOM.clientHeight)
  view.dispatch({
    selection: { anchor: at.from },
    effects: [EditorView.scrollIntoView(at.from, { y: 'start', yMargin }), markLine.of(at.from)],
  })
}
