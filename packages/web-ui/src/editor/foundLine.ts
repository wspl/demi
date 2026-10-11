import { StateEffect, StateField, type Extension } from '@codemirror/state'
import { Decoration, EditorView, type DecorationSet } from '@codemirror/view'

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
 * What lets a code view open at a line (`revealLine`): the mark goes once
 * its flash has played, so a line drawn again later does not flash again.
 */
export function foundLineExtension(): Extension {
  return [
    foundLine,
    EditorView.domEventHandlers({
      animationend(event, view) {
        if (event.animationName === 'found-highlight' && view.state.field(foundLine).size > 0)
          view.dispatch({ effects: markLine.of(null) })
      },
    }),
  ]
}

/**
 * Scrolls line `line` (1-based, held to the text's lines) to the middle of
 * the view, puts the cursor at its start, and marks it for a moment, as a
 * search result is (`file-previews.md` § Files named in messages).
 */
export function revealLine(view: EditorView, line: number): void {
  const doc = view.state.doc
  const at = doc.line(Math.min(Math.max(line, 1), doc.lines))
  view.dispatch({
    selection: { anchor: at.from },
    effects: [EditorView.scrollIntoView(at.from, { y: 'center' }), markLine.of(at.from)],
  })
}
