import { describe, expect, test } from 'bun:test'
import { nextActiveWorkTab, workTabTitle, type WorkTab } from '../work-panel'

const tabs: WorkTab[] = [
  { id: 'a', kind: 'file', path: 'src/auth/cookie.ts' },
  { id: 'b', kind: 'diff', path: 'src/auth/cookie.ts' },
  { id: 'c', kind: 'file', path: 'tests/login/auth.test.ts' },
]

describe('work panel tabs', () => {
  test('a tab is titled with the file name', () => {
    expect(workTabTitle(tabs[2]!)).toBe('auth.test.ts')
  })

  test('closing a tab activates the one before it, else the first', () => {
    expect(nextActiveWorkTab(tabs, 'b')).toBe('a')
    expect(nextActiveWorkTab(tabs, 'a')).toBe('b')
    expect(nextActiveWorkTab(tabs, 'c')).toBe('b')
    expect(nextActiveWorkTab([tabs[0]!], 'a')).toBeNull()
  })
})
