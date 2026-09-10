import { expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import { deferred } from '@demicodes/utils'
import { useMessageForks, type MessageForkRequest } from '../message-fork'

test('pending ignores double clicks, failure retries the same UUID and success permits a new Fork', async () => {
  const scope = effectScope()
  const requests: MessageForkRequest[] = []
  const release = deferred<void>()
  let fail = true
  const forks = scope.run(() => useMessageForks(() => async (request) => {
    requests.push({ ...request })
    await release.promise
    if (fail) throw new Error('Lost confirmation')
  }, () => 'source'))!
  try {
    const first = forks.run('answer')
    await forks.run('answer')
    expect(requests).toHaveLength(1)
    expect(forks.states.value.get('answer')?.phase).toBe('pending')
    release.resolve()
    await first
    expect(forks.states.value.get('answer')).toMatchObject({ phase: 'failed', error: 'Lost confirmation' })
    fail = false
    await forks.run('answer')
    expect(requests[1]).toEqual(requests[0])
    expect(forks.states.value.size).toBe(0)
    await forks.run('answer')
    expect(requests[2]!.id).not.toBe(requests[0]!.id)
  } finally {
    release.resolve()
    scope.stop()
  }
})

test('changing conversations isolates pending outcomes and message actions', async () => {
  const scope = effectScope()
  const conversation = ref('source')
  const release = deferred<void>()
  const forks = scope.run(() => useMessageForks(() => async () => {
    await release.promise
    throw new Error('late failure')
  }, () => conversation.value))!
  try {
    const first = forks.run('answer-1')
    const second = forks.run('answer-2')
    expect(forks.states.value.size).toBe(2)
    conversation.value = 'destination'
    expect(forks.states.value.size).toBe(0)
    release.resolve()
    await Promise.all([first, second])
    expect(forks.states.value.size).toBe(0)
  } finally {
    release.resolve()
    scope.stop()
  }
})
