import { expect, test } from 'bun:test'
import {
  firstEnabledSettingsTab,
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

test('the first usable tab is the first enabled rail item', () => {
  expect(firstEnabledSettingsTab()).toBe('general')
})
