import { expect, test } from 'bun:test'
import { clickedChoice } from '../segmented'

test('with two options, a click on either segment or on the frame switches to the other', () => {
  const values = ['preview', 'source'] as const
  expect(clickedChoice(values, 'preview', 'preview')).toBe('source')
  expect(clickedChoice(values, 'preview', 'source')).toBe('source')
  expect(clickedChoice(values, 'preview', undefined)).toBe('source')
  expect(clickedChoice(values, 'source', 'source')).toBe('preview')
  expect(clickedChoice(values, 'source', undefined)).toBe('preview')
})

test('with three options, a click chooses the clicked segment and the frame changes nothing', () => {
  const values = ['first', 'second', 'third'] as const
  expect(clickedChoice(values, 'first', 'first')).toBe('first')
  expect(clickedChoice(values, 'first', 'third')).toBe('third')
  expect(clickedChoice(values, 'second', undefined)).toBe('second')
})
