import { expect, spyOn, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import type { Block } from '@demicodes/core'
import type { SessionLoad } from '../session-status'
import { useChromeEntrance } from '../useChromeEntrance'

function block(id: string): Block {
  return {
    type: 'abort',
    id,
    createdAt: '2026-09-13T00:00:00.000Z',
    model: null as unknown as Extract<Block, { type: 'abort' }>['model'],
    isResumed: false,
  }
}

test('async history stays still, live blocks enter once, and changing conversations resets history', async () => {
  const scope = effectScope()
  const blocks = ref<Block[]>([])
  const load = ref<SessionLoad>('loading')
  const conversation = ref('first')
  const held = ref<string | null>(null)
  const timers = spyOn(globalThis, 'setTimeout')
  const clear = spyOn(globalThis, 'clearTimeout')
  const entrance = scope.run(() => useChromeEntrance(
    () => blocks.value,
    () => held.value,
    () => conversation.value,
    () => load.value,
  ))!
  try {
    // A response updates the history and ready state in the same Vue batch.
    blocks.value = [block('history')]
    load.value = 'ready'
    await nextTick()
    expect(entrance.isEntering('history')).toBe(false)
    expect(timers).not.toHaveBeenCalled()
    blocks.value.push(block('live'))
    await nextTick()
    expect(entrance.isEntering('live')).toBe(true)
    const finish = timers.mock.calls.at(-1)![0] as () => void
    clearTimeout(timers.mock.results.at(-1)!.value as ReturnType<typeof setTimeout>)
    finish()
    blocks.value = [block('history')]
    await nextTick()
    blocks.value.push(block('live'))
    await nextTick()
    expect(entrance.isEntering('live')).toBe(false)
    held.value = 'handoff'
    blocks.value.push(block('handoff'))
    await nextTick()
    expect(entrance.isEntering('handoff')).toBe(false)
    blocks.value.push(block('pending'))
    await nextTick()
    conversation.value = 'second'
    blocks.value = [block('other-history')]
    await nextTick()
    expect(entrance.isEntering('pending')).toBe(false)
    expect(entrance.isEntering('other-history')).toBe(false)
    expect(clear).toHaveBeenCalled()
    load.value = 'loading'
    await nextTick()
    blocks.value.push(block('staged-history'))
    await nextTick()
    load.value = 'ready'
    await nextTick()
    expect(entrance.isEntering('staged-history')).toBe(false)
    blocks.value.push(block('after-restore'))
    await nextTick()
    expect(entrance.isEntering('after-restore')).toBe(true)
    const beforeStop = clear.mock.calls.length
    scope.stop()
    expect(clear.mock.calls.length).toBe(beforeStop + 1)
  } finally {
    scope.stop()
    timers.mockRestore()
    clear.mockRestore()
  }
})
