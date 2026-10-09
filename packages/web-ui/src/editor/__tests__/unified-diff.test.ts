import { expect, test } from 'bun:test'
import { EditorState } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { getChunks, uncollapseUnchanged } from '@codemirror/merge'
import { unifiedDiff } from '../unifiedDiff'
import { codeViewUpdate } from '../useCodeView'

// Cost: two diffs of a 30 KB text without a view, under 10 ms.

/** A source file of 600 lines, about 30 KB, with `changed` lines (1-based) edited. */
function source(changed: readonly number[]): string {
  return Array.from({ length: 600 }, (_, index) => {
    const line = index + 1
    const text = `export const value${line} = compute(${line}, 'the same words on every line of the file')`
    return changed.includes(line) ? `${text} // edited` : text
  }).join('\n')
}

/** The stretches of lines the diff folds into one row, by their first and last line. */
function folded(state: EditorState): { first: number; last: number }[] {
  const out: { first: number; last: number }[] = []
  for (const set of state.facet(EditorView.decorations)) {
    if (typeof set === 'function')
      continue
    set.between(0, state.doc.length, (from, to, decoration) => {
      if (decoration.spec.block && typeof decoration.spec.widget?.lines === 'number')
        out.push({ first: state.doc.lineAt(from).number, last: state.doc.lineAt(to).number })
    })
  }
  return out.sort((a, b) => a.first - b.first)
}

const EDITED = [20, 70, 120, 170, 220, 270, 320, 370, 420, 470, 520, 570]

test('a large file edited throughout shows each edit as its own change, and a new version unfolds what it changed', () => {
  const original = source([])
  let state = EditorState.create({ doc: source(EDITED), extensions: unifiedDiff(original) })
  // Not the whole file as one replaced stretch.
  expect(getChunks(state)?.chunks.length).toBe(EDITED.length)
  expect(folded(state).some((stretch) => stretch.first <= 45 && stretch.last >= 45)).toBe(true)

  // The user unfolds the stretch above the first edit.
  const top = state.doc.line(1).from
  state = state.update({ effects: uncollapseUnchanged.of(top) }).state
  expect(folded(state)[0]!.first).toBeGreaterThan(20)

  // The file is read again with line 45, inside a folded stretch, edited too.
  state = state.update(codeViewUpdate(state, { path: 'values.ts', text: source([...EDITED, 45]), original })!).state
  expect(getChunks(state)?.chunks.length).toBe(EDITED.length + 1)
  expect(folded(state).some((stretch) => stretch.first <= 45 && stretch.last >= 45)).toBe(false)
  // What the user unfolded stays open.
  expect(folded(state)[0]!.first).toBeGreaterThan(20)
})
