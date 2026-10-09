import { RangeSetBuilder, StateField, type EditorState, type Extension } from '@codemirror/state'
import { Decoration, EditorView, WidgetType, type DecorationSet } from '@codemirror/view'
import { getChunks, uncollapseUnchanged, unifiedMergeView } from '@codemirror/merge'

/**
 * How long one diff may take, in milliseconds, before the rest of it is
 * matched roughly. The merge view's own default instead gives up on any
 * stretch of more than 16,000 changed-or-not characters and calls all of it
 * changed, so a file of 25 KB with edits spread through it showed as wholly
 * replaced, and each read of a new version swung the whole view.
 */
const DIFF_TIMEOUT_MS = 500

/** Unchanged lines kept around each change, as GitHub and git show. */
const MARGIN = 3
/** The fewest unchanged lines that fold into one row. */
const MIN_FOLDED = 4

/** A stretch of unchanged lines folded into one row that a click unfolds. */
class FoldedLines extends WidgetType {
  constructor(readonly lines: number) {
    super()
  }

  eq(other: FoldedLines): boolean {
    return other.lines === this.lines
  }

  toDOM(view: EditorView): HTMLElement {
    const row = document.createElement('div')
    // The merge view's own class, which its theme styles.
    row.className = 'cm-collapsedLines'
    row.textContent = view.state.phrase('$ unchanged lines', this.lines)
    row.addEventListener('click', (event) => {
      const at = view.posAtDOM(event.target as Node)
      view.dispatch({ effects: uncollapseUnchanged.of(at) })
    })
    return row
  }

  ignoreEvent(event: Event): boolean {
    return event instanceof MouseEvent
  }

  get estimatedHeight(): number {
    return 27
  }
}

interface Folds {
  /** The folded stretches. */
  folded: DecorationSet
  /** The stretches the user unfolded, which stay unfolded as the texts change. */
  unfolded: DecorationSet
}

const unfoldedMark = Decoration.mark({})

/** The stretches of unchanged lines between the chunks, as the merge view's own folding finds them, but those the user unfolded. */
function foldsOf(state: EditorState, unfolded: DecorationSet): DecorationSet {
  const chunks = getChunks(state)?.chunks ?? []
  const builder = new RangeSetBuilder<Decoration>()
  let previous = 1
  for (let index = 0; ; index += 1) {
    const chunk = index < chunks.length ? chunks[index] : null
    const from = index ? previous + MARGIN : 1
    const to = chunk ? state.doc.lineAt(chunk.fromB).number - 1 - MARGIN : state.doc.lines
    if (to - from + 1 >= MIN_FOLDED) {
      const start = state.doc.line(from).from
      const end = state.doc.line(to).to
      let opened = false
      unfolded.between(start, end, () => {
        opened = true
        return false
      })
      if (!opened)
        builder.add(start, end, Decoration.replace({ widget: new FoldedLines(to - from + 1), block: true }))
    }
    if (!chunk)
      break
    previous = state.doc.lineAt(Math.min(state.doc.length, chunk.toB)).number
  }
  return builder.finish()
}

/**
 * The unchanged stretches, folded, built again whenever the chunks change:
 * the merge view's own folding is built once and only moved along after,
 * so a change that a new version brought into a folded stretch stayed
 * hidden in it, counted as unchanged.
 */
const folds = StateField.define<Folds>({
  create(state) {
    return { folded: foldsOf(state, Decoration.none), unfolded: Decoration.none }
  },
  update(value, tr) {
    let unfolded = value.unfolded.map(tr.changes)
    let folded = value.folded.map(tr.changes)
    for (const effect of tr.effects) {
      if (!effect.is(uncollapseUnchanged))
        continue
      folded.between(effect.value, effect.value, (from, to) => {
        unfolded = unfolded.update({ add: [unfoldedMark.range(from, to)] })
      })
      folded = folded.update({ filter: (from) => from !== effect.value })
    }
    if (getChunks(tr.startState)?.chunks !== getChunks(tr.state)?.chunks)
      folded = foldsOf(tr.state, unfolded)
    return { folded, unfolded }
  },
  provide: (field) => EditorView.decorations.from(field, (value) => value.folded),
})

/**
 * A unified diff of the editor's text against `original`: each removed
 * stretch above what replaced it, and the unchanged lines between changes
 * folded but for three around each, a stretch of four or more into one row
 * a click unfolds. New texts of either side replace the old in place
 * (`useCodeView`), and the folding follows the chunks they make.
 */
export function unifiedDiff(original: string): Extension {
  return [
    unifiedMergeView({
      original,
      highlightChanges: true,
      gutter: false,
      mergeControls: false,
      syntaxHighlightDeletions: true,
      diffConfig: { timeout: DIFF_TIMEOUT_MS },
    }),
    folds,
  ]
}
