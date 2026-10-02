import { describe, expect, test } from 'bun:test'
import { closeTabs, tabsToClose } from '../tab-close'

describe('work panel tab closing', () => {
  const pages = [{ id: 'a' }, { id: 'b' }, { id: 'c' }]
  test('selects the nearest remaining tab and allows an empty panel', () => {
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
