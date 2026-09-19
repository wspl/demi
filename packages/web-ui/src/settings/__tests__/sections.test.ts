import { expect, test } from 'bun:test'
import {
  isSettingsSectionEnabled,
  SETTINGS_SECTIONS,
} from '../sections'

test('deferred whole pages stay listed and cannot be opened', () => {
  const ids = SETTINGS_SECTIONS.flatMap((group) => group.items).map(
    (item) => item.id,
  )
  expect(ids).toContain('data')
  expect(ids).toContain('notifications')
  expect(isSettingsSectionEnabled('data')).toBe(false)
  expect(isSettingsSectionEnabled('notifications')).toBe(false)
  expect(isSettingsSectionEnabled('mcp')).toBe(false)
  expect(isSettingsSectionEnabled('skills')).toBe(false)
  expect(isSettingsSectionEnabled('general')).toBe(true)
  expect(isSettingsSectionEnabled('account')).toBe(true)
})
