import { expect, test } from 'bun:test'
import { moveBefore } from './reorder'

test('moveBefore supports either direction and the end without mutating input', () => {
  const items = ['a', 'b', 'c']
  expect(moveBefore(items, 'c', 'a')).toEqual(['c', 'a', 'b'])
  expect(moveBefore(items, 'a', 'c')).toEqual(['b', 'a', 'c'])
  expect(moveBefore(items, 'a', null)).toEqual(['b', 'c', 'a'])
  expect(items).toEqual(['a', 'b', 'c'])
  expect(moveBefore(items, 'missing', null)).toEqual(items)
  expect(moveBefore(items, 'a', 'missing')).toEqual(items)
  expect(moveBefore(items, 'a', 'a')).toEqual(items)
})
