import { expect, test } from 'bun:test'
import type { BackendToRunnerMessage } from '@demicodes/runner-protocol'
import { memoryHostStore } from '@demicodes/shell/testing'
import { RemoteHost } from '../remote-host'

test('conversation release skips offline Hosts and joins acknowledgement without admitting work', async () => {
  const frames: BackendToRunnerMessage[] = []
  const host = new RemoteHost({
    defaultCwd: '/', identity: { uid: 1, gid: 1, hostname: 'fixture', homeDir: '/' },
    store: memoryHostStore(),
    admit: () => { throw new Error('release must not admit work') },
  })
  await host.releaseConversation('conversation')
  host.attach(message => { frames.push(message) })
  let finished = false
  const released = host.releaseConversation('conversation').then(() => { finished = true })
  await Promise.resolve()
  expect(finished).toBe(false)
  const request = frames[0]
  expect(request?.type).toBe('conversation_release')
  if (request?.type !== 'conversation_release') throw new Error('missing release')
  expect(request.conversationId).toBe('conversation')
  host.handleMessage({ type: 'conversation_released', id: request.id })
  await released
  expect(finished).toBe(true)
  for (const error of ['cleanup failed', '']) {
    const failed = host.releaseConversation('conversation')
    const next = frames.at(-1)
    if (next?.type !== 'conversation_release') throw new Error('missing release')
    host.handleMessage({ type: 'conversation_released', id: next.id, error })
    await expect(failed).rejects.toThrow(error)
  }
  const disconnected = host.releaseConversation('conversation')
  host.detach()
  await expect(disconnected).rejects.toThrow()
})
