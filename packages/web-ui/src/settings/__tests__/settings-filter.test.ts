import { expect, test } from 'bun:test'
import { SETTINGS_SECTIONS } from '../sections'
import { filterSettings, firstMatch } from '../settings-filter'

// Cost: pure functions over the product's rail; under a millisecond.
function found(query: string) {
  return filterSettings(SETTINGS_SECTIONS, query).flatMap((group) =>
    group.matches.map((match) => [match.item.id, ...match.settings.map((entry) => entry.label)]),
  )
}

test('the filter finds a setting by its label or what people call it, under its section', () => {
  expect(found('dark mode')).toEqual([['general', 'Theme']])
  expect(found('Password')).toEqual([['account', 'Password']])
  expect(found('font size')).toEqual([['general', 'Transcript text size']])
})

test('a section found by its own name lists none of its settings', () => {
  expect(found('keyboard')).toEqual([['keyboard']])
  expect(found('api key')).toEqual([['models']])
})

test('Enter opens the first setting found, skipping a section still in development', () => {
  expect(firstMatch(filterSettings(SETTINGS_SECTIONS, 'password'))).toEqual({ section: 'account', setting: 'Password' })
  expect(firstMatch(filterSettings(SETTINGS_SECTIONS, 'keyboard'))).toEqual({ section: 'keyboard', setting: null })
  // Notifications is in development: it is listed, but nothing opens.
  expect(firstMatch(filterSettings(SETTINGS_SECTIONS, 'sound'))).toBeNull()
  expect(found('nothing like this')).toEqual([])
})
