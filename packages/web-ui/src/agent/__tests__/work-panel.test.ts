import { describe, expect, test } from 'bun:test'
import { addBrowserTab, closeBrowserTabs, changeTabPath, findChangeWorkTab, goBackInTab, goForwardInTab, showCallEdit, showChangeInTab, showFileInTab, workPanelTabs } from '../work-panel'
import { closeTabs, tabsToClose } from '../tab-close'
import type { CallEditSelection } from '../../files/changes'

const selection: CallEditSelection = {
  commandId: 'call-a',
  file: { path: 'src/index.ts', kind: 'modified', added: 2, removed: 1, edits: [{ kept: true }, { kept: true }] },
}

describe('fixed work panel sections', () => {
  test('starts with only the fixed Change and File tabs', () => {
    const tabs = workPanelTabs()
    expect(tabs.map((tab) => tab.kind)).toEqual(['change', 'file'])
    expect(findChangeWorkTab(tabs)?.mode).toBe('uncommitted')
  })

  test('browser tabs have independent drafts and closing never removes fixed tabs', () => {
    const first = addBrowserTab(workPanelTabs())
    const second = addBrowserTab(first.tabs)
    expect(first.activeId).not.toBe(second.activeId)
    expect(second.tabs).toHaveLength(4)
    expect(second.tabs[3]).toMatchObject({ kind: 'browser', title: 'New tab', address: '' })
    expect(closeBrowserTabs(second.tabs, second.activeId, [second.activeId]).activeId).toBe(first.activeId)
    const closed = closeBrowserTabs(second.tabs, second.activeId, second.tabs.map((tab) => tab.id))
    expect(closed.tabs.map((tab) => tab.kind)).toEqual(['change', 'file'])
    expect(closed.activeId).toBe('file')
    expect(closeBrowserTabs(second.tabs, 'change', [first.activeId]).activeId).toBe('change')
  })

  test('files selected from any section replace the single file and preserve history', () => {
    const first = showFileInTab(workPanelTabs(), 'src/a.ts')
    const second = showFileInTab(first.tabs, 'src/b.ts')
    expect(second.activeId).toBe('file')
    expect(second.tabs).toHaveLength(2)
    expect(second.tabs[1]).toMatchObject({ path: 'src/b.ts', back: ['src/a.ts'], forward: [] })
    const back = goBackInTab(second.tabs, 'file')
    expect(back[1]).toMatchObject({ path: 'src/a.ts', back: [], forward: ['src/b.ts'] })
    expect(goForwardInTab(back, 'file')[1]).toMatchObject({ path: 'src/b.ts' })
    const replaced = showFileInTab(back, 'src/c.ts')
    expect(replaced.tabs[1]).toMatchObject({ path: 'src/c.ts', forward: [] })
    expect(showFileInTab(replaced.tabs, 'src/c.ts').tabs[1]).toEqual(replaced.tabs[1])
  })

  test('retained edits open Change and returning to Uncommitted keeps its selection', () => {
    const initial = showChangeInTab(workPanelTabs(), 'change', 'uncommitted', 'README.md')
    const retained = showCallEdit(initial, selection)
    expect(retained.activeId).toBe('change')
    expect(retained.tabs).toHaveLength(2)
    expect(findChangeWorkTab(retained.tabs)).toMatchObject({ mode: 'conversation', call: selection })
    const uncommitted = showChangeInTab(retained.tabs, 'change', 'uncommitted', 'README.md')
    expect(changeTabPath(findChangeWorkTab(uncommitted)!)).toBe('README.md')
    expect(findChangeWorkTab(goBackInTab(uncommitted, 'change'))?.mode).toBe('conversation')
  })

  test('history distinguishes calls, files and edit segments', () => {
    let tabs = showCallEdit(workPanelTabs(), selection).tabs
    tabs = showChangeInTab(tabs, 'change', 'conversation', selection.file.path, { call: selection, edit: 1 })
    const other = { ...selection, commandId: 'call-b' }
    tabs = showCallEdit(tabs, other).tabs
    expect(findChangeWorkTab(tabs)).toMatchObject({ call: other, edit: 0 })
    const back = goBackInTab(tabs, 'change')
    expect(findChangeWorkTab(back)).toMatchObject({ call: selection, edit: 1 })
    expect(findChangeWorkTab(goForwardInTab(back, 'change'))).toMatchObject({ call: other, edit: 0 })
    const nextFile = { ...other, file: { ...other.file, path: 'src/other.ts' } }
    tabs = showCallEdit(tabs, nextFile).tabs
    expect(changeTabPath(findChangeWorkTab(tabs)!)).toBe('src/other.ts')
    expect(changeTabPath(findChangeWorkTab(goBackInTab(tabs, 'change'))!)).toBe('src/index.ts')
  })
})

describe('browser tab closing', () => {
  const pages = [{ id: 'a' }, { id: 'b' }, { id: 'c' }]
  test('selects the nearest remaining tab and allows an empty browser', () => {
    expect(closeTabs(pages, 'b', ['b']).activeId).toBe('a')
    expect(closeTabs(pages, 'a', ['a']).activeId).toBe('b')
    expect(closeTabs(pages, 'a', ['a', 'b', 'c'])).toEqual({ tabs: [], activeId: null })
  })
  test('scoped menus close only their selected side', () => {
    expect(tabsToClose(pages, 'b', 'others')).toEqual(['a', 'c'])
    expect(tabsToClose(pages, 'b', 'right')).toEqual(['c'])
    expect(tabsToClose(pages, 'b', 'left')).toEqual(['a'])
    expect(closeTabs(pages, 'b', tabsToClose(pages, 'b', 'others'))).toEqual({ tabs: [{ id: 'b' }], activeId: 'b' })
  })
})
