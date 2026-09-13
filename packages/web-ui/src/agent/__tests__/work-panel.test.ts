import { describe, expect, test } from 'bun:test'
import { changeWorkTab, changeTabPath, showCallEdit, closeWorkTabs, fileWorkTab, findChangeWorkTab, goBackInTab, goForwardInTab, showChangeInTab, showFileInTab, workTabTitle, type WorkTab } from '../work-panel'
import { tabsToClose } from '../tab-close'

const tabs: WorkTab[] = [
  fileWorkTab('a', 'src/auth/cookie.ts'),
  changeWorkTab('b', 'uncommitted'),
  fileWorkTab('c', 'tests/login/auth.test.ts'),
]

describe('work panel tabs', () => {
  test('a file tab is titled with the file name, the change tab Change', () => {
    expect(workTabTitle(tabs[2]!)).toBe('auth.test.ts')
    expect(workTabTitle(tabs[1]!)).toBe('Change')
  })

  test('the change tab is found when open', () => {
    expect(findChangeWorkTab(tabs)?.id).toBe('b')
    expect(findChangeWorkTab([tabs[0]!])).toBeNull()
  })

  test('closing the active tab activates the nearest one before it, else the first', () => {
    expect(closeWorkTabs(tabs, 'b', ['b']).activeId).toBe('a')
    expect(closeWorkTabs(tabs, 'a', ['a']).activeId).toBe('b')
    expect(closeWorkTabs(tabs, 'c', ['b', 'c']).activeId).toBe('a')
    expect(closeWorkTabs(tabs, 'a', ['a', 'b', 'c'])).toEqual({ tabs: [], activeId: null })
  })

  test('closing other tabs keeps the active one', () => {
    expect(closeWorkTabs(tabs, 'b', ['a', 'c'])).toEqual({ tabs: [tabs[1]!], activeId: 'b' })
  })

  test('a close scope names the tabs on that side', () => {
    expect(tabsToClose(tabs, 'b', 'self')).toEqual(['b'])
    expect(tabsToClose(tabs, 'b', 'others')).toEqual(['a', 'c'])
    expect(tabsToClose(tabs, 'b', 'right')).toEqual(['c'])
    expect(tabsToClose(tabs, 'a', 'left')).toEqual([])
    expect(tabsToClose(tabs, 'zzz', 'others')).toEqual([])
  })

  test('a picked file replaces the active file tab in place and is remembered', () => {
    const shown = showFileInTab(tabs, 'a', 'src/auth/session.ts', () => 'new')
    expect(shown.activeId).toBe('a')
    expect(shown.tabs.map((tab) => tab.id)).toEqual(['a', 'b', 'c'])
    const active = shown.tabs[0]!
    expect(active.kind === 'file' && active.path).toBe('src/auth/session.ts')
    expect(active.kind === 'file' && active.back).toEqual(['src/auth/cookie.ts'])
  })

  test('a file already shown gets its tab; no active file tab opens one', () => {
    expect(showFileInTab(tabs, 'a', 'tests/login/auth.test.ts', () => 'new').activeId).toBe('c')
    const fromChange = showFileInTab(tabs, 'b', 'src/index.ts', () => 'new')
    expect(fromChange.activeId).toBe('new')
    expect(fromChange.tabs).toHaveLength(4)
  })

  test('back and forward walk the files a tab has shown', () => {
    let state = showFileInTab(tabs, 'a', 'src/auth/session.ts', () => 'new').tabs
    state = showFileInTab(state, 'a', 'src/index.ts', () => 'new').tabs
    const pathOf = (list: WorkTab[]) => { const tab = list[0]!; return tab.kind === 'file' ? tab.path : null }
    expect(pathOf(state)).toBe('src/index.ts')
    state = goBackInTab(state, 'a')
    expect(pathOf(state)).toBe('src/auth/session.ts')
    state = goBackInTab(state, 'a')
    expect(pathOf(state)).toBe('src/auth/cookie.ts')
    state = goBackInTab(state, 'a')
    expect(pathOf(state)).toBe('src/auth/cookie.ts')
    state = goForwardInTab(state, 'a')
    expect(pathOf(state)).toBe('src/auth/session.ts')
    // A new pick after going back drops what was ahead.
    state = showFileInTab(state, 'a', 'README.md', () => 'new').tabs
    const tab = state[0]!
    expect(tab.kind === 'file' && tab.forward).toEqual([])
  })

  test('the change tab steps through modes and files, and back across them', () => {
    const stepOf = (list: WorkTab[]) => {
      const tab = findChangeWorkTab(list)!
      return `${tab.mode}:${changeTabPath(tab)}`
    }
    let state = showChangeInTab(tabs, 'b', 'uncommitted', 'src/auth/cookie.ts')
    state = showCallEdit(state, { commandId: 'readme-call', file: { path: 'README.md', kind: 'modified', added: 1, removed: 1, edits: [{ kept: true }] } }, () => 'new').tabs
    // The same step again is not a step.
    state = showCallEdit(state, { commandId: 'readme-call', file: { path: 'README.md', kind: 'modified', added: 1, removed: 1, edits: [{ kept: true }] } }, () => 'new').tabs
    expect(stepOf(state)).toBe('conversation:README.md')
    expect(findChangeWorkTab(state)!.back).toHaveLength(2)
    state = goBackInTab(state, 'b')
    expect(stepOf(state)).toBe('uncommitted:src/auth/cookie.ts')
    // Back returns to the state before a conversation file was picked.
    expect(changeTabPath(findChangeWorkTab(state)!, 'conversation')).toBeNull()
    state = goBackInTab(state, 'b')
    expect(stepOf(state)).toBe('uncommitted:null')
    state = goForwardInTab(state, 'b')
    state = goForwardInTab(state, 'b')
    expect(stepOf(state)).toBe('conversation:README.md')
    state = goBackInTab(state, 'b')
    state = showChangeInTab(state, 'b', 'uncommitted', 'src/auth/sid.ts')
    expect(findChangeWorkTab(state)!.forward).toEqual([])
  })
})


test('history distinguishes calls and edit segments for the same path', () => {
  const a = { commandId: 'a', file: { path: '/work/file', kind: 'modified' as const, added: 1, removed: 1, edits: [{ kept: true }, { kept: true }] } }
  const b = { ...a, commandId: 'b' }
  let state = showChangeInTab(tabs, 'b', 'conversation', '/work/file', { call: a, edit: 1 })
  state = showChangeInTab(state, 'b', 'conversation', '/work/file', { call: b, edit: 0 })
  expect(findChangeWorkTab(state)?.call?.commandId).toBe('b')
  state = goBackInTab(state, 'b')
  expect(findChangeWorkTab(state)).toMatchObject({ call: a, edit: 1 })
  state = goForwardInTab(state, 'b')
  expect(findChangeWorkTab(state)).toMatchObject({ call: b, edit: 0 })
})


test('picking different files of the same call records each selection', () => {
  const first = { path: '/work/first', kind: 'modified' as const, added: 1, removed: 1, edits: [{ kept: true }] }
  const second = { ...first, path: '/work/second' }
  let state = showCallEdit([], { commandId: 'one-call', file: first }, () => 'edits').tabs
  expect(findChangeWorkTab(state)?.call).toEqual({ commandId: 'one-call', file: first })
  state = showCallEdit(state, { commandId: 'one-call', file: second }, () => 'unused').tabs
  expect(changeTabPath(findChangeWorkTab(state)!)).toBe(second.path)
  state = goBackInTab(state, 'edits')
  expect(findChangeWorkTab(state)?.call).toEqual({ commandId: 'one-call', file: first })
})
