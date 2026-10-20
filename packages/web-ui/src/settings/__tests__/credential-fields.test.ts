import { expect, test } from 'bun:test'
import { draftFields, fieldNameFor, textProblem, valueProblem, type EditorField } from '../credential-fields'

function field(label: string, value: string): EditorField {
  return { key: 0, label, kind: 'secret', value }
}

test('a new field is read by a name made from its label, different from the others', () => {
  expect(fieldNameFor('API key', [])).toBe('api-key')
  expect(fieldNameFor('  User name! ', ['user-name'])).toBe('user-name-2')
  expect(fieldNameFor('密钥', [])).toBe('field')
  expect(fieldNameFor('x'.repeat(60), []).length).toBeLessThanOrEqual(32)
})

test('the editor saves each field with its value, two fields of one label under different names', () => {
  const saved = draftFields([field('Token', 'abc'), field('Token', 'def'), field(' Host ', 'example.org')])
  expect(saved.map((each) => [each.name, each.label, each.value])).toEqual([
    ['token', 'Token', 'abc'],
    ['token-2', 'Token', 'def'],
    ['host', 'Host', 'example.org'],
  ])
})

test('a name, a label and a value say what is wrong with them', () => {
  expect(textProblem('  ', 80)).toBe('Required')
  expect(textProblem('a'.repeat(81), 80)).toBe('Use at most 80 characters.')
  expect(textProblem(' GitHub token ', 80)).toBeNull()
  expect(valueProblem('')).toBe('Required')
  expect(valueProblem('é'.repeat(40000))).toBe('Use at most 64 KiB.')
})
