import { afterEach, beforeEach, expect, test } from 'bun:test'
import { until } from '@vueuse/core'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { shallowRef } from 'vue'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { usePermissions } from './permissions'

// Cost: a played sync channel and a stubbed fetch; milliseconds.

const CONVERSATION = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
const PATH = `/api/conversations/${CONVERSATION}/permissions`
const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The answers the backend gives, in order; each read takes the next. */
let answers: Array<() => Response>
/** How many reads reached the backend, read reactively. */
const reads = shallowRef(0)
/** The backend's answer to a decision. */
let decide: () => Response

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  answers = []
  reads.value = 0
  decide = () => new Response(null, { status: 204 })
  globalThis.fetch = (async (input) => {
    if (String(input).startsWith(`${PATH}/requests/`)) {
      return decide()
    }
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
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
})

/** The product state of the backend's `run`, whose conversation's requests are at `revision`. */
function at(run: string, revision: number, waiting: number) {
  return productState({
    run,
    conversations: [
      conversationSummary(CONVERSATION, 'Skills', { permissionsRevision: revision, permissionRequests: waiting }),
    ],
  })
}

/** The read's answer: `waiting` requests of Manage skills at `revision`. */
function answer(revision: number, waiting: number): () => Response {
  return () =>
    Response.json({
      revision,
      requests: Array.from({ length: waiting }, (_, index) => ({
        id: `pr-${index}`,
        category: { id: 'skills.manage', action: 'manage skills', description: 'Manage skills.' },
        command: 'demi skills add acme/tools',
        agent: null,
        createdAt: '2026-10-05T00:00:00.000Z',
      })),
    })
}

test('after the backend started again, the page reads the requests though the new count is lower', async () => {
  const product = useProduct()
  product.start()
  channels.last().connect(at('run-1', 5, 1))
  const permissions = usePermissions()
  answers.push(answer(5, 1))
  permissions.follow(CONVERSATION)
  await until(() => permissions.stateFor(CONVERSATION).requests.length).toBe(1)

  // The backend restarts; another page decided the request just before, and
  // the new run counted one change since.
  channels.last().end(1001, 'backend_closing')
  product.reconnect()
  answers.push(answer(1, 0))
  channels.last().connect(at('run-2', 1, 0))
  await until(() => permissions.stateFor(CONVERSATION).requests.length).toBe(0)
  expect(reads.value).toBe(2)
})

test('a decision shows at once and is not read back; a refused one gives the request back', async () => {
  const product = useProduct()
  product.start()
  channels.last().connect(at('run-1', 5, 2))
  const permissions = usePermissions()
  answers.push(answer(5, 2))
  permissions.follow(CONVERSATION)
  const state = permissions.stateFor(CONVERSATION)
  await until(() => state.requests.length).toBe(2)

  await permissions.decide(CONVERSATION, 'pr-0', 'deny')
  expect(state.requests.map((request) => request.id)).toEqual(['pr-1'])
  expect(reads.value).toBe(1)

  decide = () => Response.json({ code: 'internal_error', message: 'Not decided' }, { status: 500 })
  await permissions.decide(CONVERSATION, 'pr-1', 'deny')
  expect(state.requests.map((request) => request.id)).toEqual(['pr-1'])
  expect(reads.value).toBe(1)
})
