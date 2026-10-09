import { expect, test } from 'bun:test'
import type { RequestEditSelection } from '@demicodes/plugin-sdk'
import { changePath, firstChangeData, goBack, goForward, holdShownChange, showChange, showEdit, showRequestEdit } from '../data'

// Cost: pure, under a millisecond.

const pill: RequestEditSelection = {
  node: null,
  request: 'user-1',
  file: 'src/index.ts',
  edit: { call: 'call-b', path: 'src/index.ts', segment: 0 },
}

test('a request opens Conversation, and returning to Uncommitted keeps its file', () => {
  const uncommitted = showChange(firstChangeData(), 'uncommitted', 'README.md')
  const opened = showRequestEdit(uncommitted, pill)
  expect(opened).toMatchObject({ mode: 'conversation', request: pill })
  const back = showChange(opened, 'uncommitted', opened.uncommitted)
  expect(changePath(back)).toBe('README.md')
  expect(goBack(back).mode).toBe('conversation')
})

test('the first listed file a view shows before any pick stays shown when another is listed before it', () => {
  const change = (path: string) => ({ path, kind: 'modified' as const, added: 1, removed: 1 })
  const listed = [change('src/app.ts'), change('src/main.ts')]
  const held = holdShownChange(firstChangeData(), listed)
  // A build writes a file that sorts first.
  expect(changePath(held, 'uncommitted', [change('Cargo.lock'), ...listed])).toBe('src/app.ts')
  // It shows what it showed: no step for Back.
  expect(held.back).toEqual([])
  // A view that holds a file, or shows a request, keeps what it holds.
  expect(holdShownChange(held, [change('Cargo.lock')])).toBe(held)
  const request = showRequestEdit(firstChangeData(), pill)
  expect(holdShownChange(request, listed)).toBe(request)
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
