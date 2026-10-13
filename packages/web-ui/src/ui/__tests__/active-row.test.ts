import { expect, test } from 'bun:test'
import { activeRow } from '../active-row'

test('a list taking focus rests on the first preferred row it holds, else its first row', () => {
  const rows = ['a', 'b', 'c']
  expect(activeRow(rows, 'b', 'c')).toBe('b')
  // A row that left the list, or none yet, falls to the next candidate.
  expect(activeRow(rows, 'gone', null, 'c')).toBe('c')
  expect(activeRow(rows, 'gone', undefined)).toBe('a')
  expect(activeRow([], 'a')).toBeNull()
})
