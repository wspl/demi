import { afterEach, beforeEach, expect, test } from 'bun:test'
import { until } from '@vueuse/core'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { formatBytes } from '@demicodes/web-ui/files/format'
import { conversationSummary, productState } from '../__tests__/product-state'
import { showMetadata } from '../__tests__/shown-metadata'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { usePermissions } from './permissions'
import { useConversations } from './store'
import type { ConversationPermissions } from '../api/generated/web-api'

// Cost: a played sync channel and a stubbed fetch; milliseconds.
//
// Planted defect: the requests read by a route after the metadata names a
// revision, which the fetch refuses.

const CONVERSATION = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
const PATH = `/api/conversations/${CONVERSATION}/permissions`
const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The backend's answer to a decision. */
let decide: () => Response
/** The bodies the decisions sent, in order. */
let decided: unknown[]

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  decide = () => new Response(null, { status: 204 })
  decided = []
  // The requests come with the conversation's metadata: any read of them
  // is a request the test did not expect.
  globalThis.fetch = (async (input, init) => {
    if (String(input).startsWith(`${PATH}/requests/`)) {
      decided.push(JSON.parse(String(init?.body)))
      return decide()
    }
    throw new Error(`Unexpected request: ${String(input)}`)
  }) as typeof fetch
})

afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
})

/**
 * Connects the page to the backend's `run`, whose conversation has the
 * requests of `permissions`, and shows the conversation with them.
 */
function connectAt(run: string, permissions: ConversationPermissions): void {
  channels.last().connect(productState({
    run,
    conversations: [conversationSummary(CONVERSATION, 'Skills', { permissionRequests: permissions.requests.length })],
  }))
  showMetadata(CONVERSATION, { permissions })
}

/** `waiting` requests of Manage Skills at `revision`. */
function skills(revision: number, waiting: number): ConversationPermissions {
  return {
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
  }
}

test('the requests show as the metadata brings them, and after the backend started again though the new revision is lower', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  connectAt('run-1', skills(5, 1))
  const permissions = usePermissions()
  permissions.follow(CONVERSATION)
  expect(permissions.stateFor(CONVERSATION).requests.length).toBe(1)
  // A new request reaches the open page with its value.
  showMetadata(CONVERSATION, { permissions: skills(6, 2) })
  expect(permissions.stateFor(CONVERSATION).requests.length).toBe(2)

  // The backend restarts; another page decided the requests just before, and
  // the new run counted one change since.
  channels.last().end(1001, 'backend_closing')
  product.reconnect()
  connectAt('run-2', skills(1, 0))
  await until(() => permissions.stateFor(CONVERSATION).requests.length).toBe(0)
})

test('a decision shows at once and is not read back; a refused one gives the request back', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  connectAt('run-1', skills(5, 2))
  const permissions = usePermissions()
  permissions.follow(CONVERSATION)
  const state = permissions.stateFor(CONVERSATION)
  expect(state.requests.length).toBe(2)

  await permissions.decide(CONVERSATION, 'pr-0', 'deny')
  expect(state.requests.map((request) => request.id)).toEqual(['pr-1'])

  decide = () => Response.json({ requestId: 'req_test000000000001', code: 'internal_error', message: 'Not decided' }, { status: 500 })
  await permissions.decide(CONVERSATION, 'pr-1', 'deny')
  expect(state.requests.map((request) => request.id)).toEqual(['pr-1'])
})

/** A credential request for Google's key, an allowance of a stored credential and one that was removed. */
function credentialRequests(): ConversationPermissions {
  const asked = { command: 'demi credentials …', agent: null, createdAt: '2026-10-05T00:00:00.000Z' }
  return {
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
  }
}

test('a credential request asks to update a saved credential when the backend says it renews one, and an allowance says when its credential is gone', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  // The page lists no credential of the name: the request's own word decides.
  connectAt('run-1', credentialRequests())
  const permissions = usePermissions()
  permissions.follow(CONVERSATION)
  const state = permissions.stateFor(CONVERSATION)
  expect(state.requests.length).toBe(3)

  expect(state.requests.map((request) => request.kind)).toEqual(['credential', 'allowance', 'allowance'])
  expect(state.requests[0]).toMatchObject({ name: 'Google API key', renewal: true, keep: '1h' })
  expect(state.requests[2]).toMatchObject({ name: 'Old token', removed: true })
})

test('the values of a credential go with the answer, the card stays until they are accepted, and a refusal is said in the card with the request kept', async () => {
  const product = useProduct()
  // The conversations follow the product state from its first, as on the page.
  useConversations()
  product.start()
  connectAt('run-1', credentialRequests())
  const permissions = usePermissions()
  permissions.follow(CONVERSATION)
  const state = permissions.stateFor(CONVERSATION)
  expect(state.requests.length).toBe(3)

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
