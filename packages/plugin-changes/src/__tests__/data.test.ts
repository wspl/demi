import { expect, test } from 'bun:test'
import type { RequestEditSelection } from '@demicodes/plugin-sdk'
import { changePath, firstChangeData, goBack, goForward, showChange, showEdit, showRequestEdit } from '../data'

// Cost: pure, under a millisecond.

const pill: RequestEditSelection = {
  node: null,
  request: 'user-1',
  file: 'src/index.ts',
  edit: { call: 'call-b', segment: 0 },
}

test('a request opens Conversation, and returning to Uncommitted keeps its file', () => {
  const uncommitted = showChange(firstChangeData(), 'uncommitted', 'README.md')
  const opened = showRequestEdit(uncommitted, pill)
  expect(opened).toMatchObject({ mode: 'conversation', request: pill })
  const back = showChange(opened, 'uncommitted', opened.uncommitted)
  expect(changePath(back)).toBe('README.md')
  expect(goBack(back).mode).toBe('conversation')
})

test('history tells requests, files and edits apart, and another file shows its All Changes', () => {
  let data = showRequestEdit(firstChangeData(), pill)
  data = showEdit(data, null)
  expect(data.request).toMatchObject({ file: 'src/index.ts', edit: null })
  data = showChange(data, 'conversation', 'src/other.ts')
  expect(data.request).toMatchObject({ file: 'src/other.ts', edit: null })
  const other = { ...pill, request: 'user-2' }
  data = showRequestEdit(data, other)
  expect(data.request).toEqual(other)
  const back = goBack(data)
  expect(back.request).toMatchObject({ request: 'user-1', file: 'src/other.ts' })
  expect(goBack(goBack(back)).request).toEqual(pill)
  expect(goForward(back).request).toEqual(other)
  // The same selection again is no step of its own.
  expect(showRequestEdit(data, { ...other })).toBe(data)
})
