import { expect, test } from 'bun:test'
import { defineComponent } from 'vue'
import { z } from 'zod'
import {
  addTab,
  emptyPanelState,
  moveTab,
  openIntent,
  pinnedData,
  removeTabs,
  selectedTab,
  selectInPanel,
  shownSelection,
  updateTab,
} from '../panel-tabs'
import type { PanelTabKind } from '../panel-kinds/kind'
import { definePage, type PanelKind } from '../../plugins/page'
import type { CallEditSelection } from '../../files/changes'

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
  expect(state.selection).toBeNull()
  expect(state.tabs.map((tab) => tab.id)).toEqual(ids)
  const added = addTab(state, { kind: 'browser', data: { url: 'about:blank' } }, { select: true })
  expect(added.state.selection).toBe(added.id)
  expect(selectedTab(added.state)).toEqual({ id: added.id, kind: 'browser', data: { url: 'about:blank' } })
})

test('a tab is only its id, kind and data, and its kind replaces the data', () => {
  const { state, ids } = three()
  const next = updateTab(state, ids[1]!, { url: 'b', tab: 't_1' })
  expect(next.tabs[1]).toEqual({ id: ids[1]!, kind: 'page', data: { url: 'b', tab: 't_1' } })
  expect(JSON.parse(JSON.stringify(next))).toEqual(next)
})

test('a closed selection passes to the tab before it, then the first, then to nothing', () => {
  const { state, ids } = three()
  const onLast = selectInPanel(state, ids[2]!)
  expect(removeTabs(onLast, [ids[2]!]).selection).toBe(ids[1]!)
  const onFirst = selectInPanel(state, ids[0]!)
  expect(removeTabs(onFirst, [ids[0]!]).selection).toBe(ids[1]!)
  expect(removeTabs(onFirst, ids).selection).toBeNull()
  // A pinned tab keeps the selection whatever closes.
  expect(removeTabs(selectInPanel(state, 'file'), ids)).toEqual({ selection: 'file', tabs: [] })
})

test('a tab moves before another or to the end', () => {
  const { state, ids } = three()
  expect(moveTab(state, ids[2]!, ids[0]!).tabs.map((tab) => tab.id)).toEqual([ids[2]!, ids[0]!, ids[1]!])
  expect(moveTab(state, ids[0]!, null).tabs.map((tab) => tab.id)).toEqual([ids[1]!, ids[2]!, ids[0]!])
})

const nothing = defineComponent({ setup: () => () => null })

function pinnedKind(kind: string, first: string): PanelTabKind<string> {
  return { kind, schema: z.string(), title: (data) => data, mark: nothing, content: nothing, pinned: { data: () => first } }
}

test('the panel shows its first pinned tab until the selection names something it shows', () => {
  const kinds = [pinnedKind('change', 'c'), pinnedKind('file', 'f')]
  const { state, ids } = three()
  expect(shownSelection(state, kinds)).toBe('change')
  expect(shownSelection(selectInPanel(state, 'file'), kinds)).toBe('file')
  expect(shownSelection(selectInPanel(state, ids[1]!), kinds)).toBe(ids[1])
  // A kind turned off, or a tab that is gone, names nothing the panel shows.
  expect(shownSelection(selectInPanel(state, 'gone'), kinds)).toBe('change')
  expect(shownSelection(selectInPanel(state, 'gone'), [])).toBe(ids[0])
  expect(pinnedData({}, kinds[1]!)).toBe('f')
  expect(pinnedData({ file: 'g' }, kinds[1]!)).toBe('g')
})

function fileKind(kind: string, pinned: boolean, opened: unknown[] = []): PanelKind<string> {
  return {
    kind,
    schema: z.string(),
    title: (data) => data,
    mark: nothing,
    content: nothing,
    pinned: pinned ? { data: () => '' } : undefined,
    intents: {
      file(payload, current) {
        opened.push(current)
        return `${String(current)} > ${payload.path}`
      },
    },
  }
}

test('an intent opens the first enabled page that declares it, in its pinned tab', () => {
  const opened: unknown[] = []
  const pages = [
    definePage({ plugin: 'off', kinds: [fileKind('other', true)] }),
    definePage({ plugin: 'files', kinds: [fileKind('file', true, opened)] }),
  ]
  const enabled = (plugin: string) => plugin !== 'off'
  const first = openIntent({ state: emptyPanelState(), pinned: {} }, pages, enabled, { intent: 'file', payload: { path: '/a' } })
  expect(first).toEqual({ state: { selection: 'file', tabs: [] }, pinned: { file: 'null > /a' } })
  const second = openIntent(first!, pages, enabled, { intent: 'file', payload: { path: '/b' } })
  expect(second?.pinned).toEqual({ file: 'null > /a > /b' })
  expect(opened).toEqual([null, 'null > /a'])
  // No enabled page opens `edit`: the shell shows no control for it.
  const edit: CallEditSelection = { commandId: 'c', file: { path: 'x', kind: 'added', added: 1, removed: 0, edits: [] } }
  expect(openIntent(first!, pages, enabled, { intent: 'edit', payload: edit })).toBeNull()
})

test('an intent a kind that is not pinned opens gets a new tab of its own, selected', () => {
  const pages = [definePage({ plugin: 'pages', kinds: [fileKind('page', false)] })]
  const opened = openIntent({ state: emptyPanelState(), pinned: {} }, pages, () => true, { intent: 'file', payload: { path: '/a' } })
  const tab = opened!.state.tabs[0]!
  expect(opened!.state).toEqual({ selection: tab.id, tabs: [{ id: tab.id, kind: 'page', data: 'null > /a' }] })
  expect(opened!.pinned).toEqual({})
})
