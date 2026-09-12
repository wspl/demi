import { expect, spyOn, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useElapsedTime } from '../useElapsedTime'
import { requestingFaceLabel, thinkingFaceLabel } from '../../agent/thinking-label'

test('thinking and requesting show durations only after one second', () => {
  expect(thinkingFaceLabel(false, 0)).toBe('Thought briefly')
  expect(thinkingFaceLabel(false, 999)).toBe('Thought briefly')
  expect(thinkingFaceLabel(true, 500)).toBe('Thinking')
  expect(thinkingFaceLabel(true, 1000)).toBe('Thinking')
  expect(thinkingFaceLabel(true, 1001)).toBe('Thinking for 1s')
  expect(requestingFaceLabel(0)).toBe('Requesting')
  expect(requestingFaceLabel(1000)).toBe('Requesting')
  expect(requestingFaceLabel(1001)).toBe('Requesting for 1s')
  expect(thinkingFaceLabel(false, 60000)).toBe('Thought for 1m')
})

test('live intervals tick, freeze to their persisted end, reset and release clocks', async () => {
  const clock = spyOn(Date, 'now').mockReturnValue(10000)
  const intervals = spyOn(globalThis, 'setInterval')
  const clear = spyOn(globalThis, 'clearInterval')
  const start = ref(9000)
  const end = ref<number | null>(null)
  const running = ref(true)
  const scope = effectScope()
  const elapsed = scope.run(() => useElapsedTime(
    () => start.value,
    () => running.value,
    () => end.value,
  ))!
  try {
    expect(elapsed.value).toBe(1000)
    clock.mockReturnValue(12000)
    const tick = intervals.mock.calls.at(-1)![0] as () => void
    tick()
    expect(elapsed.value).toBe(3000)
    end.value = 11500
    running.value = false
    await nextTick()
    expect(elapsed.value).toBe(2500)
    expect(clear).toHaveBeenCalledTimes(1)
    start.value = 12000
    end.value = null
    running.value = true
    await nextTick()
    expect(elapsed.value).toBe(0)
    scope.stop()
    expect(clear).toHaveBeenCalledTimes(2)
  } finally {
    scope.stop()
    clock.mockRestore()
    intervals.mockRestore()
    clear.mockRestore()
  }
})
