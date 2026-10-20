import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterAll, afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { RequestOutputReading } from '@demicodes/conversation-client'
import { LINGER_MS, ROLL_MS, SPEED_AFTER_MS, useRequestOutput, type RequestOutputDisplay } from '../request-output'

// The roll's frames come from the window's animation frames.
GlobalRegistrator.register({ url: 'https://demi.test/chat' })
afterAll(() => GlobalRegistrator.unregister())

beforeEach(() => {
  jest.useFakeTimers()
})

/** The displays of the running test, stopped at its end, whether or not it passed. */
const displays: Array<() => void> = []

afterEach(() => {
  for (const stop of displays.splice(0))
    stop()
  jest.useRealTimers()
})

/** What the server and the page tell the display. */
const told = reactive({
  output: null as RequestOutputReading | null,
  scope: 'conversation-a',
  reducedMotion: false,
})

/** A display following `told` from its present state, until it stops. */
function display(): RequestOutputDisplay & { stop: () => void } {
  const scope = effectScope()
  const shown = scope.run(() => useRequestOutput({
    output: () => told.output,
    scope: () => told.scope,
    reducedMotion: () => told.reducedMotion,
    window,
  }))!
  displays.push(() => scope.stop())
  return { ...shown, stop: () => scope.stop() }
}

function reading(requestId: string, tokens: number, firstTokenAt: number | null = Date.now(), endedAt: number | null = null): RequestOutputReading {
  return { requestId, tokens, firstTokenAt, endedAt }
}

/** Lets the display take what it was told, moves the clock by `ms` and lets the frame due then run. */
async function after(ms: number): Promise<void> {
  await nextTick()
  // Bun's clock moves a millisecond on an advance of none.
  if (ms > 0)
    jest.advanceTimersByTime(ms)
  await new Promise((resolve) => setImmediate(resolve))
  await nextTick()
}

/** The timers still set: bun forgets a cleared or fired one at its clock's next move. */
function timersSet(): number {
  jest.advanceTimersByTime(1)
  return jest.getTimerCount()
}

function reset(): void {
  told.output = null
  told.scope = 'conversation-a'
  told.reducedMotion = false
}

// The count rolls to each new value of one request over about half a
// second; a new request, another conversation's reading and reduced motion
// change it at once.
test('the count rolls within a request and changes at once otherwise', async () => {
  reset()
  told.output = reading('r-1', 100)
  const shown = display()
  expect(shown.count.value).toBe(100)

  told.output = reading('r-1', 300)
  await after(ROLL_MS / 2)
  expect(shown.count.value).toBeGreaterThan(100)
  expect(shown.count.value).toBeLessThan(300)
  await after(ROLL_MS)
  expect(shown.count.value).toBe(300)

  // The next request starts from zero, at once.
  told.output = reading('r-2', 0)
  await after(0)
  expect(shown.count.value).toBe(0)

  // Another conversation's reading, at once.
  told.output = reading('r-9', 4_000)
  told.scope = 'conversation-b'
  await after(0)
  expect(shown.count.value).toBe(4_000)

  told.reducedMotion = true
  told.output = reading('r-9', 4_200)
  await after(0)
  expect(shown.count.value).toBe(4_200)
  shown.stop()
})

// Less than a second of streaming reads as thousands of tokens a second, so
// the speed waits for the first second; while no reading arrives it follows
// the time, a second at a time, by one timer, which goes with the reading it
// was for and stops at the request's end.
test('the speed shows from the request\'s first second and follows the time, by a timer released with its reading', async () => {
  reset()
  told.output = reading('r-1', 0, null)
  const shown = display()
  expect(shown.speed.value).toBeNull()
  expect(timersSet()).toBe(0)

  told.output = reading('r-1', 40, Date.now())
  await after(0)
  expect(shown.speed.value).toBeNull()
  expect(timersSet()).toBe(1)
  await after(SPEED_AFTER_MS - 2)
  expect(shown.speed.value).toBeNull()
  await after(1)
  expect(shown.speed.value).toBe(40)
  // No reading comes for a second: the speed follows the time.
  await after(1_000)
  expect(shown.speed.value).toBe(20)

  // A response that ended keeps its speed over the time it streamed.
  const first = Date.now() - 4_000
  told.output = reading('r-2', 200, first, first + 2_000)
  await after(0)
  expect(shown.speed.value).toBe(100)
  expect(timersSet()).toBe(0)

  // A request still in its first second when the display stops leaves no timer.
  told.output = reading('r-3', 10, Date.now())
  await after(0)
  expect(timersSet()).toBe(1)
  shown.stop()
  expect(timersSet()).toBe(0)
})

// When the turn ends the last reading stays 3 seconds, then goes; a turn
// that starts meanwhile replaces it, and another conversation without one
// shows none at once.
test('the last reading lingers after the turn and its timer is released', async () => {
  reset()
  told.output = reading('r-1', 900, Date.now() - 5_000, Date.now())
  const shown = display()

  told.output = null
  await after(LINGER_MS - 1)
  expect(shown.shown.value?.tokens).toBe(900)
  await after(1)
  expect(shown.shown.value).toBeNull()

  // The response's own count comes with the turn's end, in one tick, and is
  // the reading that stays.
  told.output = reading('r-2', 64, Date.now() - 2_000)
  await after(0)
  told.output = reading('r-2', 50, Date.now() - 2_000, Date.now())
  told.output = null
  await after(ROLL_MS)
  expect([shown.shown.value?.tokens, shown.count.value]).toEqual([50, 50])
  told.output = reading('r-2', 50, Date.now() - 2_000)
  await after(0)
  told.output = null
  await after(LINGER_MS / 2)
  told.output = reading('r-3', 0, null)
  await after(LINGER_MS)
  expect(shown.shown.value?.requestId).toBe('r-3')

  told.output = null
  await after(0)
  told.scope = 'conversation-b'
  await after(0)
  expect(shown.shown.value).toBeNull()
  expect(timersSet()).toBe(0)
  shown.stop()
})
