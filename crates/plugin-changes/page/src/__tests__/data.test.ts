import { expect, test } from 'bun:test'
import type { RequestEditSelection } from '@demicodes/plugin-sdk'
import { changePath, changePlace, firstChangeData, holdShownChange, placeBranch, showBranch, showChange, showCommit, showEdit, showItem, showPresentation, showRequestEdit, viewDirectory, type ChangeData } from '../data'

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
  expect(back.request).toEqual(pill)
})

test('the first listed file a view shows before any pick stays shown when another is listed before it', () => {
  const change = (path: string) => ({ path, kind: 'modified' as const, added: 1, removed: 1 })
  const listed = [change('src/app.ts'), change('src/main.ts')]
  const held = holdShownChange(firstChangeData(), listed)
  // A build writes a file that sorts first.
  expect(changePath(held, 'commits', [change('Cargo.lock'), ...listed])).toBe('src/app.ts')
  // A view that holds a file, or shows a request, keeps what it holds.
  expect(holdShownChange(held, [change('Cargo.lock')])).toBe(held)
  const request = showRequestEdit(firstChangeData(), pill)
  expect(holdShownChange(request, listed)).toBe(request)
})

test('the view tells requests, files and edits apart, and another file shows its All Changes', () => {
  let data = showRequestEdit(firstChangeData(), pill)
  data = showEdit(data, null)
  expect(data.request).toMatchObject({ file: 'src/index.ts', edit: null })
  data = showChange(data, 'conversation', 'src/other.ts')
  expect(data.request).toMatchObject({ file: 'src/other.ts', edit: null })
  const other = { ...pill, request: 'user-2' }
  data = showRequestEdit(data, other)
  expect(data.request).toEqual(other)
  // The same selection again changes nothing.
  expect(showRequestEdit(data, { ...other })).toBe(data)
})

test('a request of another directory is another selection: the same file of two directories is not the view shown', () => {
  const child = { ...pill, root: '/slots/2' }
  const data = showRequestEdit(firstChangeData(), pill)
  const moved = showRequestEdit(data, child)
  expect(moved.request).toEqual(child)
  expect(moved).not.toBe(data)
  expect(showRequestEdit(moved, { ...child })).toBe(moved)
})

test('a commit link opens Commits on it in its agent’s directory, its list placed once', () => {
  const request = showRequestEdit(firstChangeData(), { ...pill, root: '/slots/2' })
  // A link the main agent wrote reads the conversation's directory, whatever the request's.
  const linked = showCommit(request, { commit: '3f2a9c1' })
  expect(linked).toMatchObject({ mode: 'commits', commit: '3f2a9c1', file: null, place: true })
  expect(viewDirectory(linked)).toBe(undefined)
  // A child's link reads its directory.
  expect(viewDirectory(showCommit(request, { commit: '3f2a9c1', root: '/slots/3' }))).toBe('/slots/3')
  const placed = placeBranch(linked, 'feat/cart')
  expect(placed).toMatchObject({ branch: 'feat/cart', place: false })
  expect(showBranch(placed, 'feat/cart')).toBe(placed)
  expect(showItem(placed, null)).toMatchObject({ commit: null, file: null, branch: 'feat/cart' })
})

// A reload, a copied link and Back show the Change view where its address
// says (`docs/decisions/0014-the-work-panel-has-addresses.md`): what the place
// writes reads back as the same view. Planted defect this catches: a field the
// place writes and does not read, such as a commit link's directory or an
// edit's renamed path.
test('the Change view reads back from its address where it was, and a place that does not read shows its start', () => {
  const renamed = { ...pill, file: 'src/main.ts', edit: { call: 'call-c', path: 'src/index.ts', segment: 2 }, root: '/slots/2' }
  const views: ChangeData[] = [
    firstChangeData(),
    showChange(firstChangeData(), 'commits', 'README.md'),
    showRequestEdit(firstChangeData(), pill),
    showPresentation(showRequestEdit(firstChangeData(), renamed), 'preview'),
    showEdit(showRequestEdit(firstChangeData(), { ...pill, node: 'agent-2' }), null),
    showCommit(showRequestEdit(firstChangeData(), pill), { commit: '3f2a9c1' }),
    showBranch(showCommit(firstChangeData(), { commit: '3f2a9c1', root: '/slots/3' }), 'feat/cart'),
  ]
  for (const view of views) {
    // A change is written with the view it shows in: its own, or the page's last choice.
    expect(changePlace.read(changePlace.write(view), view)).toEqual({ ...view, view: view.view ?? 'diff' })
  }
  expect(changePlace.write(views[2]!)).toEqual({
    mode: 'conversation',
    request: 'user-1',
    path: 'src/index.ts',
    call: 'call-b',
    segment: '0',
    view: 'diff',
  })
  // The commit list is placed once, for the commit the link opened.
  expect(changePlace.read(changePlace.write(views[1]!), views[5]!)).toMatchObject({ place: false })
  expect(changePlace.read({ segment: '-1' }, firstChangeData())).toBeNull()
  // Another change names no view: it shows in the page's choice until the tab records it.
  expect(showChange(views[3]!, 'commits', 'README.md').view).toBeUndefined()
  expect(showPresentation(views[3]!, 'preview')).toBe(views[3])
})
