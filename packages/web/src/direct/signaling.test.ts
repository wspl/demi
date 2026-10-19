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

/**
 * The page's two peers share the socket (`direct-channel.md` § Making the
 * channel): each message names its peer, and each answer, candidate and
 * close reaches the peer it names. Planted defect this catches: one peer's
 * answer or candidate taken by the other.
 */
test('each peer\'s offer, answer and candidates go by the name of its peer', async () => {
  const closed: string[] = []
  const signaling = new DeviceSignaling(LAPTOP, () => {}, (peer) => closed.push(peer), () => {})
  const socket = channels.opened[0]!
  socket.open()
  const files = signaling.peer('files')
  const live = signaling.peer('live')
  const heard: string[] = []
  files.candidates((candidate) => heard.push(`files ${candidate}`))
  live.candidates((candidate) => heard.push(`live ${candidate}`))
  const filesAnswer = files.offer('files offer')
  const liveAnswer = live.offer('live offer')
  live.candidate('c1')
  signaling.close('live')
  expect(socket.fromPage).toEqual([
    { type: 'offer', peer: 'files', sdp: 'files offer' },
    { type: 'offer', peer: 'live', sdp: 'live offer' },
    { type: 'candidate', peer: 'live', candidate: 'c1' },
    { type: 'close', peer: 'live' },
  ])
  await expect(liveAnswer).rejects.toThrow('closed the peer')
  const message = (value: unknown) => socket.deliver(value)
  message({ type: 'candidate', peer: 'live', candidate: 'r1' })
  message({ type: 'answer', peer: 'files', sdp: 'files answer' })
  message({ type: 'closed', peer: 'live' })
  expect(await filesAnswer).toBe('files answer')
  expect(heard).toEqual(['live r1'])
  expect(closed).toEqual(['live'])
  signaling.stop()
})
