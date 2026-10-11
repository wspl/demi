import { expect, test } from 'bun:test'
import { EditorState } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { uncollapseUnchanged } from '@codemirror/merge'
import { diffOf, unifiedDiff } from '../unifiedDiff'
import { codeViewUpdate } from '../useCodeView'

// Cost: two diffs of a 30 KB text and a few of a few lines, without a view, under 20 ms.

/**
 * The rows the diff shows for each change, as git prints them: each removed
 * line, then each added line, and a note after a last line without a
 * newline.
 */
/** Where git writes "\\ No newline at end of file". */
const NO_NEWLINE = 'no newline'

function rows(original: string, modified: string): string[] {
  const state = EditorState.create({ doc: modified, extensions: unifiedDiff(original) })
  const diff = diffOf(state)!
  const out: string[] = []
  for (const chunk of diff.chunks) {
    if (chunk.toA > chunk.fromA)
      out.push(...diff.original.sliceString(chunk.fromA, chunk.endA).split('\n').map((line) => `-${line}`))
    if (chunk.toA > diff.original.length)
      out.push(NO_NEWLINE)
    if (chunk.toB > chunk.fromB)
      out.push(...state.doc.sliceString(chunk.fromB, chunk.endB).split('\n').map((line) => `+${line}`))
    if (chunk.toB > state.doc.length)
      out.push(NO_NEWLINE)
  }
  return out
}

test.each([
  // `printf 'x' > page.txt`: no removed row above the added line.
  ['a created file without a final newline', '', 'x', ['+x', NO_NEWLINE]],
  ['a created file', '', 'x\ny\n', ['+x', '+y']],
  ['a removed file', 'x\ny\n', '', ['-x', '-y']],
  ['a removed file without a final newline', 'x', '', ['-x', NO_NEWLINE]],
  // No empty added row below what is left.
  ['a deleted last line that had no newline', 'a\nb', 'a\n', ['-b', NO_NEWLINE]],
  ['a deleted last line', 'a\nb\n', 'a\n', ['-b']],
  ['a final newline removed', 'a\nb\n', 'a\nb', ['-b', '+b', NO_NEWLINE]],
  ['a final newline added', 'a\nb', 'a\nb\n', ['-b', NO_NEWLINE, '+b']],
  ['a line added after one without a newline', 'a', 'a\nb\n', ['-a', NO_NEWLINE, '+a', '+b']],
  ['a last line changed, neither with a newline', 'a\nb', 'a\nc', ['-b', NO_NEWLINE, '+c', NO_NEWLINE]],
  ['an empty line removed', 'a\n\nb\n', 'a\nb\n', ['-']],
])('%s shows git’s rows', (_, original, modified, expected) => {
  expect(rows(original, modified)).toEqual(expected)
})

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
  expect(diffOf(state)?.chunks.length).toBe(EDITED.length)
  expect(folded(state).some((stretch) => stretch.first <= 45 && stretch.last >= 45)).toBe(true)

  // The user unfolds the stretch above the first edit.
  const top = state.doc.line(1).from
  state = state.update({ effects: uncollapseUnchanged.of(top) }).state
  expect(folded(state)[0]!.first).toBeGreaterThan(20)

  // The file is read again with line 45, inside a folded stretch, edited too.
  state = state.update(codeViewUpdate(state, { path: 'values.ts', text: source([...EDITED, 45]), original })!).state
  expect(diffOf(state)?.chunks.length).toBe(EDITED.length + 1)
  expect(folded(state).some((stretch) => stretch.first <= 45 && stretch.last >= 45)).toBe(false)
  // What the user unfolded stays open.
  expect(folded(state)[0]!.first).toBeGreaterThan(20)
})
