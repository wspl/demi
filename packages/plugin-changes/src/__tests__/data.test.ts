import { expect, test } from 'bun:test'
import type { RequestEditSelection } from '@demicodes/plugin-sdk'
import { changePath, firstChangeData, goBack, goForward, holdShownChange, placeBranch, showBranch, showChange, showCommit, showEdit, showItem, showRequestEdit, viewDirectory } from '../data'

// Cost: pure, under a millisecond.

const pill: RequestEditSelection = {
  node: null,
  request: 'user-1',
  file: 'src/index.ts',
  edit: { call: 'call-b', path: 'src/index.ts', segment: 0 },
}

test('a request opens Conversation, and returning to Commits keeps its file', () => {
  const uncommitted = showChange(firstChangeData(), 'commits', 'README.md')
  const opened = showRequestEdit(uncommitted, pill)
  expect(opened).toMatchObject({ mode: 'conversation', request: pill })
  const back = showChange(opened, 'commits', opened.file)
  expect(changePath(back)).toBe('README.md')
  expect(goBack(back).mode).toBe('conversation')
})

test('the first listed file a view shows before any pick stays shown when another is listed before it', () => {
  const change = (path: string) => ({ path, kind: 'modified' as const, added: 1, removed: 1 })
  const listed = [change('src/app.ts'), change('src/main.ts')]
  const held = holdShownChange(firstChangeData(), listed)
  // A build writes a file that sorts first.
  expect(changePath(held, 'commits', [change('Cargo.lock'), ...listed])).toBe('src/app.ts')
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

test('a request of another directory is another selection: the same file of two directories is not the view shown', () => {
  const child = { ...pill, root: '/slots/2' }
  const data = showRequestEdit(firstChangeData(), pill)
  const moved = showRequestEdit(data, child)
  expect(moved.request).toEqual(child)
  expect(goBack(moved).request).toEqual(pill)
  expect(showRequestEdit(moved, { ...child })).toBe(moved)
})

test('a commit link opens Commits on it in its agent’s directory, and Back returns to what was shown', () => {
  const request = showRequestEdit(firstChangeData(), { ...pill, root: '/slots/2' })
  // A link the main agent wrote reads the conversation's directory, whatever the request's.
  const linked = showCommit(request, { commit: '3f2a9c1' })
  expect(linked).toMatchObject({ mode: 'commits', commit: '3f2a9c1', file: null, place: true })
  expect(viewDirectory(linked)).toBe(undefined)
  // A child's link reads its directory.
  expect(viewDirectory(showCommit(request, { commit: '3f2a9c1', root: '/slots/3' }))).toBe('/slots/3')
  // The list is placed once, and choosing a branch is no step of its own.
  const placed = placeBranch(linked, 'feat/cart')
  expect(placed).toMatchObject({ branch: 'feat/cart', place: false })
  expect(showBranch(placed, 'main').back).toEqual(placed.back)
  // Selecting an item is a step; Back returns to the link's commit, then to the request with its directory.
  const item = showItem(placed, null)
  expect(item).toMatchObject({ commit: null, file: null })
  expect(goBack(item)).toMatchObject({ commit: '3f2a9c1' })
  const first = goBack(goBack(item))
  expect(first).toMatchObject({ mode: 'conversation', request: { root: '/slots/2' } })
  expect(viewDirectory(first)).toBe('/slots/2')
})
