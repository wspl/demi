import { expect, spyOn, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useElapsedTime } from '../useElapsedTime'
import { useRelativeTime, useResetTime } from '../useRelativeTime'
import { providerWaitLabel, thinkingFaceLabel } from '../../agent/thinking-label'

test('thinking and requesting show durations only after one second', () => {
  expect(thinkingFaceLabel(false, 0)).toBe('Thought briefly')
  expect(thinkingFaceLabel(false, 999)).toBe('Thought briefly')
  expect(thinkingFaceLabel(true, 500)).toBe('Thinking')
  expect(thinkingFaceLabel(true, 1000)).toBe('Thinking')
  expect(thinkingFaceLabel(true, 1001)).toBe('Thinking for 1s')
  expect(providerWaitLabel(0)).toBe('Requesting')
  expect(providerWaitLabel(1000)).toBe('Requesting')
  expect(providerWaitLabel(1001)).toBe('Requesting for 1s')
  expect(providerWaitLabel(42_000)).toBe('Requesting for 42s')
  expect(thinkingFaceLabel(false, 60000)).toBe('Thought for 1m')
  expect(thinkingFaceLabel(false, 134_900)).toBe('Thought for 2m 14s')
  expect(providerWaitLabel(3_900_000)).toBe('Requesting for 1h 5m')
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

// Every running row reads one clock: the second row adds no timer, and the
// timer goes only when the last row stops running or unmounts.
test('running rows share one clock, released by the last of them', () => {
  const intervals = spyOn(globalThis, 'setInterval')
  const clear = spyOn(globalThis, 'clearInterval')
  const first = effectScope()
  const second = effectScope()
  try {
    first.run(() => useElapsedTime(() => 0, () => true))
    second.run(() => useElapsedTime(() => 0, () => true))
    expect(intervals).toHaveBeenCalledTimes(1)
    first.stop()
    expect(clear).not.toHaveBeenCalled()
    second.stop()
    expect(clear).toHaveBeenCalledTimes(1)
  } finally {
    first.stop()
    second.stop()
    intervals.mockRestore()
    clear.mockRestore()
  }
})

// A running command's time and the relative times on the page ("3 minutes
// ago", a reset countdown) read one clock, so the app keeps one timer.
test('elapsed and relative times share one timer, released when none reads it', () => {
  const intervals = spyOn(globalThis, 'setInterval')
  const clear = spyOn(globalThis, 'clearInterval')
  const running = effectScope()
  const relative = effectScope()
  const reset = effectScope()
  try {
    running.run(() => useElapsedTime(() => 0, () => true))
    relative.run(() => useRelativeTime('2026-10-07T10:30:00.000Z'))
    reset.run(() => useResetTime('2026-10-07T10:30:00.000Z'))
    expect(intervals).toHaveBeenCalledTimes(1)
    running.stop()
    relative.stop()
    expect(clear).not.toHaveBeenCalled()
    reset.stop()
    expect(clear).toHaveBeenCalledTimes(1)
  } finally {
    running.stop()
    relative.stop()
    reset.stop()
    intervals.mockRestore()
    clear.mockRestore()
  }
})
