import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { TYPE_SELECT_PAUSE_MS, useTypeSelect } from '../useTypeSelect'

// The rows as a tree or list shows them, top first.
const names = ['.git', 'docs', 'package.json', 'packages', 'Pages', 'src']

function press(key: string) {
  return { key, ctrlKey: false, metaKey: false, altKey: false }
}

function setup() {
  const selected: string[] = []
  const typeSelect = useTypeSelect({
    names: () => names,
    select: (index) => selected.push(names[index]!),
  })
  const type = (keys: string) => [...keys].map((key) => typeSelect.keydown(press(key)))
  return { typeSelect, selected, type }
}

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

test('typing selects the first row from the top whose name starts with the query, case ignored', () => {
  const { typeSelect, selected, type } = setup()
  type('pa')
  // `package.json` and `packages` both start with `pa`; the first shown wins over `Pages`.
  expect(selected).toEqual(['package.json', 'package.json'])
  type('ckages')
  expect(selected.at(-1)).toBe('packages')
  expect(typeSelect.prefix('packages')).toEqual([0, 1, 2, 3, 4, 5, 6, 7])
  type('x')
  // Nothing starts with `packagesx`: the selection stays, and the query shows as unmatched.
  expect(selected.at(-1)).toBe('packages')
  expect(typeSelect.query.value).toBe('packagesx')
  expect(typeSelect.matched.value).toBe(false)
})

test('a pause ends the query and the next key starts a new one', () => {
  const { typeSelect, selected, type } = setup()
  type('d')
  jest.advanceTimersByTime(TYPE_SELECT_PAUSE_MS - 1)
  type('o')
  expect(typeSelect.query.value).toBe('do')
  jest.advanceTimersByTime(TYPE_SELECT_PAUSE_MS)
  expect(typeSelect.query.value).toBe('')
  type('s')
  expect(typeSelect.query.value).toBe('s')
  expect(selected.at(-1)).toBe('src')
})

test('Space and Backspace keep their own meaning until a query is live', () => {
  const { typeSelect, selected, type } = setup()
  expect(typeSelect.keydown(press(' '))).toBe(false)
  expect(typeSelect.keydown(press('Backspace'))).toBe(false)
  type('pac')
  expect(typeSelect.keydown(press(' '))).toBe(true)
  expect(typeSelect.query.value).toBe('pac ')
  expect(typeSelect.keydown(press('Backspace'))).toBe(true)
  expect(typeSelect.keydown(press('Backspace'))).toBe(true)
  expect(typeSelect.query.value).toBe('pa')
  expect(selected.at(-1)).toBe('package.json')
  // Taking back the last letter ends the query; Backspace is the host's again.
  type('')
  typeSelect.keydown(press('Backspace'))
  typeSelect.keydown(press('Backspace'))
  expect(typeSelect.query.value).toBe('')
  expect(typeSelect.keydown(press('Backspace'))).toBe(false)
})

test('Escape ends a live query, and keys with a modifier or a name are the host\'s', () => {
  const { typeSelect, type } = setup()
  expect(typeSelect.keydown(press('Escape'))).toBe(false)
  type('s')
  expect(typeSelect.keydown(press('Escape'))).toBe(true)
  expect(typeSelect.query.value).toBe('')
  expect(typeSelect.keydown({ ...press('a'), metaKey: true })).toBe(false)
  expect(typeSelect.keydown(press('ArrowDown'))).toBe(false)
  expect(typeSelect.keydown(press('Enter'))).toBe(false)
  expect(typeSelect.query.value).toBe('')
})
