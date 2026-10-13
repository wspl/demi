import { afterEach, beforeEach, expect, test } from 'bun:test'
import { playChannels } from '../__tests__/sync-channel'
import { DeviceSignaling } from './signaling'

// A device's signaling socket tells the page when it drops, since the
// backend closes the runner's peer then (`web-application.md` § What a
// reopened socket brings back): the page leaves the direct path at once,
// not when the peer's own channels fail.

const LAPTOP = 'b5c6d7e8-0000-4000-8000-000000000001'
let channels: ReturnType<typeof playChannels>

beforeEach(() => {
  channels = playChannels()
})

afterEach(() => {
  channels.restore()
})

test('a signaling socket that closes after it opened tells the page at once, and one the page stopped does not', () => {
  const heard: string[] = []
  const signaling = new DeviceSignaling(
    LAPTOP,
    () => heard.push('opened'),
    () => heard.push('peer closed'),
    () => heard.push('dropped'),
  )
  channels.opened[0]!.open()
  expect(heard).toEqual(['opened'])

  channels.opened[0]!.end()
  expect(heard).toEqual(['opened', 'dropped'])

  signaling.stop()
  expect(heard).toEqual(['opened', 'dropped'])
})
