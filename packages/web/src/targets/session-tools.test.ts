import { expect, test } from 'bun:test'
import { sessionToolsExposes } from './session-tools'

const devices = [
  {
    id: 'laptop',
    name: 'laptop',
    kind: 'user' as const,
    platform: 'darwin',
    claimedAt: '2026-01-01T00:00:00.000Z',
    lastSeenAt: null,
    online: true,
    home: null,
  },
  {
    id: 'managed',
    name: 'managed-7f3',
    kind: 'managed' as const,
    platform: 'linux',
    claimedAt: '2026-01-01T00:00:00.000Z',
    lastSeenAt: null,
    online: true,
    home: null,
  },
]

function expose(id: string, deviceId: string) {
  return {
    id,
    deviceId,
    address: '127.0.0.1:5173',
    url: `https://${id}.expose.demi.example/`,
    createdAt: '2026-09-17T00:00:00.000Z',
    expiresAt: '2026-09-17T00:59:00.000Z',
  }
}

test('rows keep the snapshot order and name the host: a device by name, the Cloud by product name, an unknown device by id', () => {
  const rows = sessionToolsExposes(
    [expose('a', 'managed'), expose('b', 'laptop'), expose('c', 'gone')],
    devices,
  )
  expect(rows.map((row) => [row.id, row.hostName])).toEqual([
    ['a', 'Cloud'],
    ['b', 'laptop'],
    ['c', 'gone'],
  ])
  expect(rows[0]).toEqual({
    id: 'a',
    address: '127.0.0.1:5173',
    hostName: 'Cloud',
    url: 'https://a.expose.demi.example/',
    expiresAt: '2026-09-17T00:59:00.000Z',
  })
})
