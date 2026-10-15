import { expect, test } from 'bun:test'
import { settingsSections } from '../sections'
import { filterSettings, firstMatch } from '../settings-filter'

// Cost: pure functions over the product's rail; under a millisecond.
function found(query: string) {
  return filterSettings(settingsSections(), query).flatMap((group) =>
    group.matches.map((match) => [match.item.id, ...match.settings.map((entry) => entry.label)]),
  )
}

test('the filter finds a setting by its label or what people call it, under its section', () => {
  expect(found('dark mode')).toEqual([['general', 'Theme']])
  expect(found('Password')).toEqual([['account', 'Password']])
  expect(found('font size')).toEqual([['general', 'Message text size']])
})

test('a section found by its own name lists none of its settings', () => {
  expect(found('keyboard')).toEqual([['keyboard']])
  expect(found('api key')).toEqual([['models']])
  // Providers is still found by the word its old name had.
  expect(found('models')).toEqual([['models']])
})

test('Enter opens the first setting found, skipping a section still in development', () => {
  expect(firstMatch(filterSettings(settingsSections(), 'password'))).toEqual({ section: 'account', setting: 'Password' })
  expect(firstMatch(filterSettings(settingsSections(), 'keyboard'))).toEqual({ section: 'keyboard', setting: null })
  // Data & Privacy is in development: it is listed, but nothing opens.
  expect(firstMatch(filterSettings(settingsSections(), 'usage data'))).toBeNull()
  expect(found('nothing like this')).toEqual([])
})
