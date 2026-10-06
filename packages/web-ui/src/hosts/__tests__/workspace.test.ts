import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useDevicePairing } from '../../devices/pairing'
import {
  openingChoice,
  usePairedSelection,
  type WorkspaceDevice,
} from '../workspace'

// Pure functions over two small arrays and one stubbed claim: well under a millisecond.

const devices: WorkspaceDevice[] = [
  { id: 'laptop', name: 'laptop', state: 'online' },
  { id: 'build-01', name: 'build-01', state: 'offline' },
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

test('a device paired from beside the device menu is selected there once the menu lists it', async () => {
  const listed = ref<WorkspaceDevice[]>([...devices])
  const selected = ref('laptop')
  const scope = effectScope()
  const pairing = scope.run(() => {
    const paired = usePairedSelection(
      () => listed.value,
      (id) => {
        selected.value = id
      },
    )
    const pairing = useDevicePairing(async () => ({
      ok: true,
      device: { id: 'studio', name: 'studio' },
    }))
    pairing.open(paired)
    return pairing
  })!
  pairing.phase.value = { kind: 'code' }
  await pairing.submit('ABCD-1234')
  await nextTick()
  // The claim has answered, but the page has not brought the device yet.
  expect(selected.value).toBe('laptop')

  listed.value = [...listed.value, { id: 'studio', name: 'studio', state: 'online' }]
  await nextTick()
  expect(selected.value).toBe('studio')
  scope.stop()
})
