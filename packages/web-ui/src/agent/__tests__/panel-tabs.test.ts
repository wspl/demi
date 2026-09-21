import { expect, test } from 'bun:test'
import {
  addTab,
  emptyPanelState,
  moveTab,
  panelStateSchema,
  removeTabs,
  selectedTab,
  selectInPanel,
  updateTab,
} from '../panel-tabs'

function three() {
  let state = emptyPanelState()
  const ids: string[] = []
  for (const url of ['a', 'b', 'c']) {
    const added = addTab(state, { kind: 'page', data: { url } }, { select: false })
    state = added.state
    ids.push(added.id)
  }
  return { state, ids }
}

test('a new tab stands after the others and takes the selection only when asked', () => {
  const { state, ids } = three()
  expect(state.selection).toBe('change')
  expect(state.tabs.map((tab) => tab.id)).toEqual(ids)
  const added = addTab(state, { kind: 'browser', data: { url: 'about:blank' } }, { select: true })
  expect(added.state.selection).toBe(added.id)
  expect(selectedTab(added.state)).toEqual({ id: added.id, kind: 'browser', data: { url: 'about:blank' } })
})

test('a tab is only its id, kind and data, and its kind replaces the data', () => {
  const { state, ids } = three()
  const next = updateTab(state, ids[1]!, { url: 'b', tab: 't_1' })
  expect(next.tabs[1]).toEqual({ id: ids[1]!, kind: 'page', data: { url: 'b', tab: 't_1' } })
  expect(panelStateSchema.parse(JSON.parse(JSON.stringify(next)))).toEqual(next)
})

test('a closed selection passes to the tab before it, then the first, then Change', () => {
  const { state, ids } = three()
  const onLast = selectInPanel(state, ids[2]!)
  expect(removeTabs(onLast, [ids[2]!]).selection).toBe(ids[1]!)
  const onFirst = selectInPanel(state, ids[0]!)
  expect(removeTabs(onFirst, [ids[0]!]).selection).toBe(ids[1]!)
  expect(removeTabs(onFirst, ids).selection).toBe('change')
  // A fixed view keeps the selection whatever closes.
  expect(removeTabs(selectInPanel(state, 'file'), ids)).toEqual({ selection: 'file', tabs: [] })
})

test('a tab moves before another or to the end', () => {
  const { state, ids } = three()
  expect(moveTab(state, ids[2]!, ids[0]!).tabs.map((tab) => tab.id)).toEqual([ids[2]!, ids[0]!, ids[1]!])
  expect(moveTab(state, ids[0]!, null).tabs.map((tab) => tab.id)).toEqual([ids[1]!, ids[2]!, ids[0]!])
})
