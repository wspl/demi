import { expect, test } from 'bun:test'
import { openingChoice, type WorkspaceDevice } from '../workspace'

// Pure function over two small arrays: well under a millisecond.

const devices: WorkspaceDevice[] = [
  { id: 'laptop', name: 'laptop', online: true },
  { id: 'build-01', name: 'build-01', online: false },
]

test('New project opens on the Cloud the first time, with no device chosen', () => {
  expect(openingChoice(undefined, devices)).toEqual({ kind: 'cloud' })
})

test('New project opens on the last choice of kind and device, offline or not', () => {
  expect(openingChoice({ kind: 'device', deviceId: 'build-01' }, devices)).toEqual({
    kind: 'device',
    deviceId: 'build-01',
  })
  // The device last chosen stays while the Cloud is chosen, for a switch to Device.
  expect(openingChoice({ kind: 'cloud', deviceId: 'laptop' }, devices)).toEqual({
    kind: 'cloud',
    deviceId: 'laptop',
  })
})

test('a device the user no longer has leaves Device with no device chosen', () => {
  expect(openingChoice({ kind: 'device', deviceId: 'removed' }, devices)).toEqual({
    kind: 'device',
  })
})
