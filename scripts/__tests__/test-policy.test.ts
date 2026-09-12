import { expect, test } from 'bun:test'
import { externalTestEnabled } from '../test-policy'

test('offline regression mode overrides inherited external-service opt-ins', () => {
  expect(externalTestEnabled('DEMI_CODEX_E2E', {})).toBe(false)
  expect(externalTestEnabled('DEMI_CODEX_E2E', { DEMI_CODEX_E2E: 'true' })).toBe(false)
  expect(externalTestEnabled('DEMI_CODEX_E2E', { DEMI_CODEX_E2E: '1' })).toBe(true)
  expect(externalTestEnabled('DEMI_CODEX_E2E', {
    DEMI_TEST_MODE: 'offline', DEMI_CODEX_E2E: '1',
  })).toBe(false)
  expect(externalTestEnabled('DEMI_FIRECRACKER_E2E', {
    DEMI_TEST_MODE: 'offline', DEMI_FIRECRACKER_E2E: '1',
  })).toBe(false)
})
