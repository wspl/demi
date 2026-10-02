import { describe, expect, test } from 'bun:test'
import { TAB_HISTORY, activeTab, closeTabs, selectTab, tabsToClose } from '../tab-close'

describe('work panel tab closing', () => {
  const pages = [{ id: 'a' }, { id: 'b' }, { id: 'c' }, { id: 'd' }]
  const ids = (tabs: readonly { id: string }[]) => tabs.map((tab) => tab.id)

  test('closing the active tab activates the one selected before it, and the one before that if it closed too', () => {
    // The user went a, then c, then b.
    const history = ['a', 'c', 'b'].reduce<string[]>(selectTab, [])
    expect(activeTab(ids(pages), history)).toBe('b')
    let next = closeTabs(pages, history, ['b'])
    expect(activeTab(ids(next.tabs), next.history)).toBe('c')
    // c closes while it is not active, then the active a closes: no tab selected before it is left.
    next = closeTabs(next.tabs, selectTab(next.history, 'a'), ['c'])
    expect(activeTab(ids(next.tabs), next.history)).toBe('a')
    next = closeTabs(next.tabs, next.history, ['a'])
    expect(next).toEqual({ tabs: [{ id: 'd' }], history: [] })
    // With no selection left, the first tab is active, and an empty strip has none.
    expect(activeTab(ids(next.tabs), next.history)).toBe('d')
    expect(activeTab([], next.history)).toBeNull()
  })

  test('a tab selected again moves to the newest end, and the history stays bounded', () => {
    expect(['a', 'b', 'a'].reduce<string[]>(selectTab, [])).toEqual(['b', 'a'])
    const many = Array.from({ length: TAB_HISTORY + 5 }, (_, index) => `t${index}`).reduce<string[]>(selectTab, [])
    expect(many).toHaveLength(TAB_HISTORY)
    expect(many.at(-1)).toBe(`t${TAB_HISTORY + 4}`)
  })

  test('scoped menus close only their selected side', () => {
    expect(tabsToClose(pages, 'b', 'others')).toEqual(['a', 'c', 'd'])
    expect(tabsToClose(pages, 'b', 'right')).toEqual(['c', 'd'])
    expect(tabsToClose(pages, 'b', 'left')).toEqual(['a'])
    const others = closeTabs(pages, ['a', 'b'], tabsToClose(pages, 'b', 'others'))
    expect(others).toEqual({ tabs: [{ id: 'b' }], history: ['b'] })
  })
})
