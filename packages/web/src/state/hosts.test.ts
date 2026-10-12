import { expect, test } from 'bun:test'
import { cloudDeviceIn, hostKindOf, hostLabel } from './hosts'

const cloud = { id: 'c', kind: 'managed' as const, name: 'cloud-box' }
const laptop = { id: 'l', kind: 'user' as const, name: 'Laptop' }

test('the Cloud is the managed device, called Cloud whatever its name, and a missing device is unavailable', () => {
  expect(cloudDeviceIn([laptop, cloud])).toBe(cloud)
  expect(cloudDeviceIn([laptop])).toBeUndefined()
  expect(hostKindOf(cloud)).toBe('cloud')
  expect(hostKindOf(laptop)).toBe('device')
  expect(hostLabel(hostKindOf(cloud), cloud)).toBe('Cloud')
  expect(hostLabel(hostKindOf(laptop), laptop)).toBe('Laptop')
  expect(hostLabel(hostKindOf(null), null)).toBe('Unavailable device')
})
