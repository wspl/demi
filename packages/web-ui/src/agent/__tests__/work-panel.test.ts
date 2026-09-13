import { describe, expect, test } from 'bun:test'
import { closeWorkTabs, workTabTitle, type WorkTab } from '../work-panel'
import { tabsToClose } from '../tab-close'

const tabs: WorkTab[] = [
  { id: 'a', kind: 'file', path: 'src/auth/cookie.ts' },
  { id: 'b', kind: 'change', path: 'src/auth/cookie.ts' },
  { id: 'c', kind: 'file', path: 'tests/login/auth.test.ts' },
]

describe('work panel tabs', () => {
  test('a tab is titled with the file name', () => {
    expect(workTabTitle(tabs[2]!)).toBe('auth.test.ts')
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
})
