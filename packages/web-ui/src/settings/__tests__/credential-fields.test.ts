import { expect, test } from 'bun:test'
import { draftFields, fieldNameFor, textProblem, valueProblem, type EditorField } from '../credential-fields'

function field(label: string, saved: string | null = null, value = ''): EditorField {
  return { key: 0, saved, label, kind: 'secret', value }
}

test('a new field is read by a name made from its label, different from the others', () => {
  expect(fieldNameFor('API key', [])).toBe('api-key')
  expect(fieldNameFor('  User name! ', ['user-name'])).toBe('user-name-2')
  expect(fieldNameFor('密钥', [])).toBe('field')
  expect(fieldNameFor('x'.repeat(60), []).length).toBeLessThanOrEqual(32)
})

test('the editor saves a field left empty as the saved one and names the new fields after the saved ones', () => {
  const saved = draftFields([field('Token', 'token'), field('Token', null, 'abc'), field('Host', 'host', 'example.org')])
  expect(saved.map((each) => [each.name, each.value])).toEqual([
    ['token', null],
    ['token-2', 'abc'],
    ['host', 'example.org'],
  ])
})

test('a name, a label and a value say what is wrong with them', () => {
  expect(textProblem('  ', 80)).toBe('Required')
  expect(textProblem('a'.repeat(81), 80)).toBe('Use at most 80 characters.')
  expect(textProblem(' GitHub token ', 80)).toBeNull()
  expect(valueProblem('', false)).toBe('Required')
  expect(valueProblem('', true)).toBeNull()
  expect(valueProblem('é'.repeat(40000), false)).toBe('Use at most 64 KiB.')
})
