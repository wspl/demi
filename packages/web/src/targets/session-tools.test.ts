import { expect, test } from 'bun:test'
import { hostName } from './session-tools'

const devices = [
  {
    id: 'laptop',
    name: 'laptop',
    kind: 'user' as const,
    platform: 'darwin' as const,
    claimedAt: '2026-01-01T00:00:00.000Z',
    lastSeenAt: null,
    online: true,
    home: null,
  },
  {
    id: 'managed',
    name: 'managed-7f3',
    kind: 'managed' as const,
    platform: 'linux' as const,
    claimedAt: '2026-01-01T00:00:00.000Z',
    lastSeenAt: null,
    online: true,
    home: null,
  },
]

test('a device is named by its name, the Cloud by the product name, an unknown device by its id', () => {
  expect(['managed', 'laptop', 'gone'].map((id) => hostName(devices, id))).toEqual([
    'Cloud',
    'laptop',
    'gone',
  ])
})
