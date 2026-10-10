import { expect, test } from 'bun:test'
import { defineComponent } from 'vue'
import { z } from 'zod'
import {
  keptContents,
  openIntent,
  pinnedData,
  removeTabs,
  selectedTab,
  shownSelection,
  type PanelState,
  type PinnedTabs,
  type ShownPanel,
} from '../panel-tabs'
import type { PanelTabKind } from '../panel-kinds/kind'
import { definePage, type PanelKind } from '../../plugins/page'
import type { RequestEditSelection } from '../../files/request-changes'

/** Three tabs, which the page selected in `history`. */
function three(history: readonly string[] = []): PanelState {
  return { history, tabs: ['a', 'b', 'c'].map((id) => ({ id, kind: 'page', data: { url: id } })) }
}

const nothing = defineComponent({ setup: () => () => null })

function pinnedKind(kind: string, first: string): PanelTabKind<string> {
  return { kind, schema: z.string(), title: (data) => data, mark: nothing, content: nothing, pinned: { data: () => first } }
}

test('the panel shows the newest selection it still shows, else its first pinned tab', () => {
  const kinds = [pinnedKind('change', 'c'), pinnedKind('file', 'f')]
  expect(shownSelection(three(), kinds)).toBe('change')
  expect(shownSelection(three(['b', 'file']), kinds)).toBe('file')
  expect(shownSelection(three(['file', 'b']), kinds)).toBe('b')
  expect(selectedTab(three(['file', 'b']), kinds)).toEqual({ id: 'b', kind: 'page', data: { url: 'b' } })
  expect(selectedTab(three(['b', 'file']), kinds)).toBeNull()
  // A kind turned off, or a tab another page closed, gives way to what was selected before it.
  expect(shownSelection(three(['b', 'gone']), kinds)).toBe('b')
  expect(shownSelection(three(['file']), [])).toBe('a')
  expect(pinnedData({}, kinds[1]!)).toBe('f')
  expect(pinnedData({ file: 'g' }, kinds[1]!)).toBe('g')
})

test('closing the shown tab shows the one selected before it, then the first pinned tab', () => {
  const kinds = [pinnedKind('change', 'c'), pinnedKind('file', 'f')]
  // The user went to File, then c, then a.
  const closedA = removeTabs(three(['file', 'c', 'a']), ['a'])
  expect(shownSelection(closedA, kinds)).toBe('c')
  const closedC = removeTabs(closedA, ['c'])
  expect(shownSelection(closedC, kinds)).toBe('file')
  // Without a selection left, the first pinned tab; a pinned tab is never closed.
  expect(shownSelection(removeTabs(three(['b']), ['b']), kinds)).toBe('change')
  expect(removeTabs(three(['file']), ['a', 'b', 'c'])).toEqual({ history: ['file'], tabs: [] })
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

/** A closed panel with no tabs, whose pinned tabs show `pinned`. */
function closed(pinned: PinnedTabs): ShownPanel {
  return { open: false, panel: { history: [], tabs: [] }, pinned }
}

test('an intent opens the first enabled page that declares it, in its pinned tab', () => {
  const opened: unknown[] = []
  const pages = [
    definePage({ plugin: 'off', kinds: [fileKind('other', true)] }),
    definePage({ plugin: 'files', kinds: [fileKind('file', true, opened)] }),
  ]
  const enabled = (plugin: string) => plugin !== 'off'
  const first = openIntent(closed({}), pages, enabled, { intent: 'file', payload: { path: '/a' } })
  expect(first).toEqual({ action: 'open', selection: 'file', pinned: { file: 'null > /a' }, created: null })
  const shown = first?.action === 'open' ? first.pinned : {}
  const second = openIntent(closed(shown), pages, enabled, { intent: 'file', payload: { path: '/b' } })
  expect(second).toMatchObject({ pinned: { file: 'null > /a > /b' } })
  expect(opened).toEqual([null, 'null > /a'])
  // No enabled page opens `edit`: the shell shows no control for it.
  const edit: RequestEditSelection = { node: null, request: 'u', file: 'x', edit: null }
  expect(openIntent(closed(shown), pages, enabled, { intent: 'edit', payload: edit })).toBeNull()
})

test('an intent a kind that is not pinned opens gets a new tab of its own, selected', () => {
  const pages = [definePage({ plugin: 'pages', kinds: [fileKind('page', false)] })]
  const opened = openIntent(closed({}), pages, () => true, { intent: 'file', payload: { path: '/a' } })
  const tab = opened?.action === 'open' ? opened.created! : null
  expect(opened).toEqual({ action: 'open', selection: tab!.id, pinned: {}, created: { id: tab!.id, kind: 'page', data: 'null > /a' } })
})

test('a content once shown stays until its tab leaves the panel', () => {
  const steps: Array<[string, readonly string[], string | null, readonly string[], readonly string[]]> = [
    ['the first selection is kept', [], 'change', ['change', 'file', 'a'], ['change']],
    ['another selection joins it', ['change'], 'a', ['change', 'file', 'a'], ['change', 'a']],
    ['selecting a kept one again changes nothing', ['change', 'a'], 'change', ['change', 'file', 'a'], ['change', 'a']],
    ['a closed tab leaves', ['change', 'a'], 'change', ['change', 'file'], ['change']],
    ['nothing selected keeps what stays', ['change', 'a'], null, ['change', 'a'], ['change', 'a']],
  ]
  for (const [step, kept, selection, present, next] of steps) {
    expect({ step, kept: keptContents(kept, selection, present) }).toEqual({ step, kept: next })
  }
})
