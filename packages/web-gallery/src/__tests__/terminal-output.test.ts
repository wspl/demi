import { expect, test } from 'bun:test'
import { DEMO_BUN_TEST, DEMO_GIT_DIFF, DEMO_RG } from '../fixtures/terminal-output'

test('demo jobs carry the CSI colors the xterm specimen shows', () => {
  expect(DEMO_BUN_TEST).toContain('\x1b[92m')
  expect(DEMO_BUN_TEST).toContain('\x1b[91m')
  expect(DEMO_RG).toContain('\x1b[35m')
  expect(DEMO_GIT_DIFF).toContain('\x1b[32m')
  expect(DEMO_GIT_DIFF).toContain('\x1b[31m')
  expect(DEMO_GIT_DIFF).toContain('\x1b[36m')
})
