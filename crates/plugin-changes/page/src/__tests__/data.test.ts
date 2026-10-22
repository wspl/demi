import { expect, test } from 'bun:test'
import type { RequestEditSelection } from '@demicodes/plugin-sdk'
import { changePlace, firstChangeData, showBranch, showChange, showCommit, showEdit, showPresentation, showRequestEdit, type ChangeData } from '../data'

// Cost: pure, under a millisecond.

const pill: RequestEditSelection = {
  node: null,
  request: 'user-1',
  file: 'src/index.ts',
  edit: { call: 'call-b', path: 'src/index.ts', segment: 0 },
}

test('a request of another directory is another selection: the same file of two directories is not the view shown', () => {
  const child = { ...pill, root: '/slots/2' }
  const data = showRequestEdit(firstChangeData(), pill)
  const moved = showRequestEdit(data, child)
  expect(moved.request).toEqual(child)
  expect(moved).not.toBe(data)
  expect(showRequestEdit(moved, { ...child })).toBe(moved)
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
