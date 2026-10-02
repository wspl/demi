import { expect, test } from 'bun:test'
import type { CallEditSelection } from '@demicodes/plugin-sdk'
import { changePath, firstChangeData, goBack, goForward, showCallEdit, showChange } from '../data'

// Cost: pure, under a millisecond.

const selection: CallEditSelection = {
  commandId: 'call-a',
  file: { path: 'src/index.ts', kind: 'modified', added: 2, removed: 1, edits: [{}, {}] },
}

test('a retained edit opens Conversation, and returning to Uncommitted keeps its file', () => {
  const uncommitted = showChange(firstChangeData(), 'uncommitted', 'README.md')
  const retained = showCallEdit(uncommitted, selection)
  expect(retained).toMatchObject({ mode: 'conversation', call: selection })
  const back = showChange(retained, 'uncommitted', retained.uncommitted)
  expect(changePath(back)).toBe('README.md')
  expect(goBack(back).mode).toBe('conversation')
})

test('history tells calls, files and edit segments apart', () => {
  let data = showCallEdit(firstChangeData(), selection)
  data = showChange(data, 'conversation', selection.file.path, { call: selection, edit: 1 })
  const other = { ...selection, commandId: 'call-b' }
  data = showCallEdit(data, other)
  expect(data).toMatchObject({ call: other, edit: 0 })
  const back = goBack(data)
  expect(back).toMatchObject({ call: selection, edit: 1 })
  expect(goForward(back)).toMatchObject({ call: other, edit: 0 })
  const nextFile = { ...other, file: { ...other.file, path: 'src/other.ts' } }
  data = showCallEdit(data, nextFile)
  expect(changePath(data)).toBe('src/other.ts')
  expect(changePath(goBack(data))).toBe('src/index.ts')
})
