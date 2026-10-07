import { afterEach, beforeEach, expect, test } from 'bun:test'
import { shallowRef } from 'vue'
import { deferred } from '@demicodes/utils'
import { until } from '@vueuse/core'
import type { StateFeed } from '@demicodes/web-ui/plugins/page'
import type { ProductState } from '../api/generated/web-api'
import { conversationSummary, productState } from '../__tests__/product-state'
import { conversationStates, type Reach } from './states'

// Cost: a stubbed fetch and a reactive snapshot; milliseconds.

const CONVERSATION = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
const PATH = `/api/conversations/${CONVERSATION}/plugins/browser/state`
const realFetch = globalThis.fetch
/** Sends at once: these tests reach the backend; the product host's waiting is its own test's. */
const direct: Reach = (send) => send()
/** The answers the backend gives, in order; each read takes the next. */
let answers: Array<() => Promise<Response>>
/** How many reads reached the backend, read reactively. */
const reads = shallowRef(0)

beforeEach(() => {
  answers = []
  reads.value = 0
  globalThis.fetch = (async (input) => {
    if (String(input) !== PATH) {
      throw new Error(`Unexpected request: ${String(input)}`)
    }
    reads.value += 1
    const answer = answers.shift()
    if (!answer) {
      throw new Error('The test gave no answer for this read')
    }
    return answer()
  }) as typeof fetch
})

afterEach(() => {
  globalThis.fetch = realFetch
})

/** Waits until `feed` holds `state`. */
async function shows(feed: StateFeed, state: unknown): Promise<void> {
  await until(() => feed.value()).toMatch((value) => Bun.deepEquals(value, state))
}

function answer(revision: number, tabs: string[]): () => Promise<Response> {
  return async () => Response.json({ revision, state: { tabs } })
}

/** The product state of the backend's `run` with the conversation's summary carrying `revision` of the browser's state. */
function at(revision: number, run = 'run-1'): ProductState {
  return productState({
    run,
    conversations: [conversationSummary(CONVERSATION, 'Work', { pluginRevisions: [{ plugin: 'browser', revision }] })],
  })
}

test("a followed state is read when the summary's revision rises", async () => {
  const snapshot = shallowRef(at(2))
  const states = conversationStates(() => snapshot.value, direct)
  answers.push(answer(2, ['t1']))
  const feed = states.follow('browser', CONVERSATION)
  await shows(feed, { tabs: ['t1'] })

  answers.push(answer(3, ['t1', 't2']))
  snapshot.value = at(3)
  await shows(feed, { tabs: ['t1', 't2'] })
  expect(reads.value).toBe(2)
  feed.stop()
})

test('a failed read keeps the last state and says why, until a read succeeds', async () => {
  const snapshot = shallowRef(at(1))
  const states = conversationStates(() => snapshot.value, direct)
  answers.push(answer(1, ['t1']))
  const feed = states.follow('browser', CONVERSATION)
  await shows(feed, { tabs: ['t1'] })

  answers.push(async () => Response.json({ code: 'device_offline', message: 'The device is offline' }, { status: 409 }))
  snapshot.value = at(2)
  await until(() => feed.error()).not.toBeNull()
  expect(feed.error()?.reason).toBe('device_offline')
  expect(feed.value()).toEqual({ tabs: ['t1'] })

  answers.push(answer(2, ['t2']))
  feed.read()
  await shows(feed, { tabs: ['t2'] })
  expect(feed.error()).toBeNull()
  feed.stop()
})

test('after the backend started again, its count is read whatever the revision held', async () => {
  const snapshot = shallowRef(at(5))
  const states = conversationStates(() => snapshot.value, direct)
  answers.push(answer(5, ['t1']))
  const feed = states.follow('browser', CONVERSATION)
  await shows(feed, { tabs: ['t1'] })

  // The new run counted five changes too: the same number, another state.
  answers.push(answer(5, []))
  snapshot.value = at(5, 'run-2')
  await shows(feed, { tabs: [] })
  expect(reads.value).toBe(2)
  feed.stop()
})

test('nothing is read once nothing follows the state, and following again shows what was read at once', async () => {
  const snapshot = shallowRef(at(1))
  const states = conversationStates(() => snapshot.value, direct)
  answers.push(answer(1, ['t1']))
  const first = states.follow('browser', CONVERSATION)
  await shows(first, { tabs: ['t1'] })
  first.stop()

  // Nothing follows the state: the rise reads nothing.
  snapshot.value = at(2)

  const pending = deferred<Response>()
  answers.push(() => pending.promise)
  const again = states.follow('browser', CONVERSATION)
  expect(again.value()).toEqual({ tabs: ['t1'] })
  // Following again reads the revision it missed.
  await until(reads).toBe(2)
  pending.resolve(Response.json({ revision: 2, state: { tabs: ['t2'] } }))
  await shows(again, { tabs: ['t2'] })
  expect(reads.value).toBe(2)
  again.stop()
})
