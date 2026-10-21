import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { reactive } from 'vue'

// A conversation's last hour of cost as cards keep it current
// (`docs/decisions/0041-the-cost-shows-the-last-hour.md`): the page reads it
// when the first card shows it, after each change of the conversation's cost
// and once a minute, the cards of a conversation share one watch, and the last
// card's end drops it. The source's answers are played by the test, on fake
// timers; milliseconds.

const { LAST_HOUR_READ_MS, createLastHourWatch } = await import('./last-hour')
const { NO_COST } = await import('@demicodes/web-ui/agent/context-cost')
type Conversation = import('../state/types').Conversation
type LastHourCost = import('../api/generated/web-api').LastHourCost

beforeEach(() => {
  jest.useFakeTimers()
})
afterEach(() => {
  jest.useRealTimers()
})

/** A conversation as far as the watch reads it. */
function conversation(): Conversation {
  return reactive({
    id: 'c1',
    cost: NO_COST,
    subagents: [{ cost: NO_COST }],
    metadata: { shown: { value: { otherCost: NO_COST } } },
    lastHour: null,
  }) as unknown as Conversation
}

/** An answer naming the root's `calls`. */
function answer(calls: number): LastHourCost {
  return { agents: [{ agent: 0, cost: { ...NO_COST, calls }, rate: null }] }
}

/** A source whose reads wait for the test to answer them, newest last. */
function source() {
  const reads: { signal: AbortSignal, answer: (value: LastHourCost) => void }[] = []
  const failures: unknown[] = []
  return {
    reads,
    failures,
    read: (_id: string, signal: AbortSignal) => new Promise<LastHourCost>((resolve) => reads.push({ signal, answer: resolve })),
    failed: (error: unknown) => failures.push(error),
  }
}

/** Lets answered reads reach their handlers. */
const settled = () => Promise.resolve().then(() => Promise.resolve())

test('a watch reads when it starts, after a change of the cost and every minute, and keeps what it read when it ends', async () => {
  const kept = source()
  const watch = createLastHourWatch(kept)
  const shown = conversation()
  const end = watch(shown)
  expect(kept.reads).toHaveLength(1)
  kept.reads[0]!.answer(answer(1))
  await settled()
  expect(shown.lastHour).toEqual(answer(1))

  // A call of the root, and of a subagent, are each a change of cost.
  shown.cost = { ...NO_COST, calls: 2 }
  await settled()
  expect(kept.reads).toHaveLength(2)
  shown.subagents[0]!.cost = { ...NO_COST, calls: 1 }
  await settled()
  expect(kept.reads).toHaveLength(3)
  kept.reads[2]!.answer(answer(3))
  await settled()

  // Nothing new in a minute: the hour has moved on all the same.
  jest.advanceTimersByTime(LAST_HOUR_READ_MS)
  expect(kept.reads).toHaveLength(4)
  kept.reads[3]!.answer(answer(4))
  await settled()
  expect(shown.lastHour).toEqual(answer(4))

  // What else of the conversation changes reads nothing.
  shown.subagents[0] = { ...shown.subagents[0]!, cost: { ...shown.subagents[0]!.cost } }
  ;(shown.metadata.shown as { value: unknown }).value = { otherCost: { ...NO_COST } }
  await settled()
  expect(kept.reads).toHaveLength(4)

  // Ending stops the reads and leaves the answer, which the next card shows
  // until its own read comes back.
  end()
  expect(shown.lastHour).toEqual(answer(4))
  expect(kept.reads.every((read) => read.signal.aborted)).toBe(true)
  jest.advanceTimersByTime(2 * LAST_HOUR_READ_MS)
  shown.cost = { ...NO_COST, calls: 9 }
  await settled()
  expect(kept.reads).toHaveLength(4)
})

test('the cards of a conversation share one watch, which ends with the last of them', async () => {
  const kept = source()
  const watch = createLastHourWatch(kept)
  const shown = conversation()
  const composer = watch(shown)
  const window = watch(shown)
  expect(kept.reads).toHaveLength(1)
  composer()
  composer()
  kept.reads[0]!.answer(answer(1))
  await settled()
  // The composer's card is gone, and its repeated end counts once: the
  // window's card still shows the hour.
  expect(shown.lastHour).toEqual(answer(1))
  jest.advanceTimersByTime(LAST_HOUR_READ_MS)
  expect(kept.reads).toHaveLength(2)
  window()
  jest.advanceTimersByTime(LAST_HOUR_READ_MS)
  expect(kept.reads).toHaveLength(2)
})

test('an answer that a later read has already replaced is dropped', async () => {
  const kept = source()
  const shown = conversation()
  createLastHourWatch(kept)(shown)
  shown.cost = { ...NO_COST, calls: 2 }
  await settled()
  expect(kept.reads).toHaveLength(2)
  kept.reads[1]!.answer(answer(2))
  await settled()
  kept.reads[0]!.answer(answer(1))
  await settled()
  expect(shown.lastHour).toEqual(answer(2))
})
