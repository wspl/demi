import { afterEach, beforeEach, expect, test } from 'bun:test'
import { until } from '@vueuse/core'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { shallowRef } from 'vue'
import { formatBytes } from '@demicodes/web-ui/files/format'
import { conversationSummary, productState } from '../__tests__/product-state'
import { showMetadata } from '../__tests__/shown-metadata'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { usePermissions } from './permissions'
import { useConversations } from './store'

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
/** The bodies the decisions sent, in order. */
let decided: unknown[]

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  answers = []
  reads.value = 0
  decide = () => new Response(null, { status: 204 })
  decided = []
  globalThis.fetch = (async (input, init) => {
    if (String(input).startsWith(`${PATH}/requests/`)) {
      decided.push(JSON.parse(String(init?.body)))
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

/**
 * Connects the page to the backend's `run`, whose conversation has `waiting`
 * requests, and shows the conversation with its requests at `revision`.
 */
function connectAt(run: string, revision: number, waiting: number): void {
  channels.last().connect(productState({
    run,
    conversations: [conversationSummary(CONVERSATION, 'Skills', { permissionRequests: waiting })],
  }))
  showMetadata(CONVERSATION, { permissionsRevision: revision })
}

/** The read's answer: `waiting` requests of Manage Skills at `revision`. */
function answer(revision: number, waiting: number): () => Response {
  return () =>
    Response.json({
      revision,
      requests: Array.from({ length: waiting }, (_, index) => ({
        kind: 'permission',
        id: `pr-${index}`,
        categories: [{ id: 'skills.manage', action: 'manage skills', description: 'Manage Skills.' }],
        host: null,
        command: 'demi skills add acme/tools',
        agent: null,
        createdAt: '2026-10-05T00:00:00.000Z',
      })),
    })
}

test('after the backend started again, the page reads the requests though the new count is lower', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  connectAt('run-1', 5, 1)
  const permissions = usePermissions()
  answers.push(answer(5, 1))
  permissions.follow(CONVERSATION)
  await until(() => permissions.stateFor(CONVERSATION).requests.length).toBe(1)

  // The backend restarts; another page decided the request just before, and
  // the new run counted one change since.
  channels.last().end(1001, 'backend_closing')
  product.reconnect()
  answers.push(answer(1, 0))
  connectAt('run-2', 1, 0)
  await until(() => permissions.stateFor(CONVERSATION).requests.length).toBe(0)
  expect(reads.value).toBe(2)
})

test('a decision shows at once and is not read back; a refused one gives the request back', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  connectAt('run-1', 5, 2)
  const permissions = usePermissions()
  answers.push(answer(5, 2))
  permissions.follow(CONVERSATION)
  const state = permissions.stateFor(CONVERSATION)
  await until(() => state.requests.length).toBe(2)

  await permissions.decide(CONVERSATION, 'pr-0', 'deny')
  expect(state.requests.map((request) => request.id)).toEqual(['pr-1'])
  expect(reads.value).toBe(1)

  decide = () => Response.json({ requestId: 'req_test000000000001', code: 'internal_error', message: 'Not decided' }, { status: 500 })
  await permissions.decide(CONVERSATION, 'pr-1', 'deny')
  expect(state.requests.map((request) => request.id)).toEqual(['pr-1'])
  expect(reads.value).toBe(1)
})

test('a page that opens a conversation while its metadata arrives reads the requests once', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  connectAt('run-1', 4, 1)
  const permissions = usePermissions()
  let release!: () => void
  const held = new Promise<void>((resolve) => { release = resolve })
  answers.push(() => answer(5, 1)())
  const fetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    await held
    return fetch(input, init)
  }) as typeof fetch
  permissions.follow(CONVERSATION)
  // The metadata of the same revision arrives while the read is on its way.
  showMetadata(CONVERSATION, { permissionsRevision: 5 })
  await Promise.resolve()
  release()
  await until(() => permissions.stateFor(CONVERSATION).requests.length).toBe(1)
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(reads.value).toBe(1)
})

/** A read's answer holding a credential request for Google's key, an allowance of a stored credential and one that was removed. */
function credentialRequests(): () => Response {
  const asked = { command: 'demi credentials …', agent: null, createdAt: '2026-10-05T00:00:00.000Z' }
  return () =>
    Response.json({
      revision: 5,
      requests: [
        {
          kind: 'credential',
          id: 'cr-1',
          name: 'Google API key',
          reason: 'To add Google as a provider',
          fields: [{ name: 'key', label: 'API key', kind: 'secret' }],
          keep: '1h',
          renewal: true,
          ...asked,
        },
        { kind: 'allowance', id: 'al-1', name: 'GitHub token', removed: false, ...asked },
        { kind: 'allowance', id: 'al-2', name: 'Old token', removed: true, ...asked },
      ],
    })
}

test('a credential request asks to update a saved credential when the backend says it renews one, and an allowance says when its credential is gone', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  // The page lists no credential of the name: the request's own word decides.
  connectAt('run-1', 5, 3)
  const permissions = usePermissions()
  answers.push(credentialRequests())
  permissions.follow(CONVERSATION)
  const state = permissions.stateFor(CONVERSATION)
  await until(() => state.requests.length).toBe(3)

  expect(state.requests.map((request) => request.kind)).toEqual(['credential', 'allowance', 'allowance'])
  expect(state.requests[0]).toMatchObject({ name: 'Google API key', renewal: true, keep: '1h' })
  expect(state.requests[2]).toMatchObject({ name: 'Old token', removed: true })
})

test('the values of a credential go with the answer, the card stays until they are accepted, and a refusal is said in the card with the request kept', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  connectAt('run-1', 5, 3)
  const permissions = usePermissions()
  answers.push(credentialRequests())
  permissions.follow(CONVERSATION)
  const state = permissions.stateFor(CONVERSATION)
  await until(() => state.requests.length).toBe(3)

  decide = () => Response.json({ requestId: 'req_test000000000001', code: 'invalid_answer', message: 'The value of “API key” can be at most 64 KiB.', params: { field: 'API key', limitBytes: 65_536 } }, { status: 400 })
  await permissions.decide(CONVERSATION, 'cr-1', 'allow', { values: { key: 'AIza' } })
  expect(state.requests.map((request) => request.id)).toEqual(['cr-1', 'al-1', 'al-2'])
  // The user corrects the value in the card: no request to trace.
  expect({ ...state.error }).toEqual({ message: `The value of “API key” can be at most ${formatBytes(65_536)}.`, requestId: null })
  expect(state.deciding).toBe(false)

  decide = () => new Response(null, { status: 204 })
  await permissions.decide(CONVERSATION, 'cr-1', 'allow', { values: { key: 'AIza' } })
  expect(state.requests.map((request) => request.id)).toEqual(['al-1', 'al-2'])
  expect(state.error).toBeNull()
  expect(decided).toEqual([
    { decision: 'allow', credential: { values: { key: 'AIza' } } },
    { decision: 'allow', credential: { values: { key: 'AIza' } } },
  ])

  await permissions.decide(CONVERSATION, 'al-2', 'deny')
  expect(state.requests.map((request) => request.id)).toEqual(['al-1'])
  expect(decided.at(-1)).toEqual({ decision: 'deny' })
})
