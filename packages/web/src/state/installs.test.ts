import { expect, test } from 'bun:test'
import type { DeviceDto } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { conversationInstalls } from './installs'

// Cost: pure, under a millisecond.

const LAPTOP = 'b5c6d7e8-0000-4000-8000-000000000001'
const CLOUD = 'b5c6d7e8-0000-4000-8000-000000000002'

function device(id: string, kind: DeviceDto['kind'], installs: DeviceDto['installs']): DeviceDto {
  return {
    id,
    kind,
    name: id === CLOUD ? 'Cloud' : 'laptop',
    platform: 'linux',
    claimedAt: '2026-09-10T00:00:00.000Z',
    lastSeenAt: null,
    online: true,
    home: '/home/ada',
    installs,
    installed: [],
  }
}

const file = { package: 'demi.file', name: 'program', version: '0.1.3', phase: 'download' as const, done: 2, total: 9 }
const cli = { package: 'demi.claude-code', name: 'Claude Code', version: '2.1.278', phase: 'download' as const, done: 40, total: 120 }
const chrome = {
  package: 'demi.browser',
  name: 'Chrome for Testing',
  version: '153.0.8010.36',
  phase: 'download' as const,
  done: 120,
  total: 196,
}

test("a conversation shows everything its Host installs and only its provider's CLI on the Cloud", () => {
  const state = productState({
    devices: [device(LAPTOP, 'user', [file]), device(CLOUD, 'managed', [chrome, cli])],
    cloud: {
      device: { id: CLOUD, name: 'Cloud' },
      state: 'running',
      operation: null,
      error: null,
      volumes: null,
      limits: { systemBytes: 16 * 1024 ** 3, homeBytes: 32 * 1024 ** 3 },
      newerImage: false,
    },
  })
  // On the laptop with Claude Code: the laptop's install, and the Cloud's CLI but not its Chrome.
  expect(conversationInstalls(state, LAPTOP, 'demi.claude-code')).toEqual([file, cli])
  // On the laptop with an HTTP provider: the laptop's alone.
  expect(conversationInstalls(state, LAPTOP, null)).toEqual([file])
  // On the Cloud with Claude Code: each of the Cloud's installs once.
  expect(conversationInstalls(state, CLOUD, 'demi.claude-code')).toEqual([chrome, cli])
})
