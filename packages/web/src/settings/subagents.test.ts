import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import type { SubagentProfile } from '../api/generated/web-api'
import { useProduct } from '../state/product'
import { useSubagentSettings } from './subagents'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** Each request the page made: its method, path and body. */
let sent: { method: string; path: string; body: unknown }[]

const reviewer: SubagentProfile = {
  id: 'p-reviewer',
  name: 'reviewer',
  description: 'Reviews a change.',
  model: null,
  instructions: null,
  canSpawn: true,
  enabled: true,
}

const explore: SubagentProfile = {
  id: 'p-explore',
  name: 'explore',
  description: 'Finds code.',
  model: { providerId: 'work', modelId: 'haiku', thinkingEffort: 'low', serviceTierId: null },
  instructions: 'You only report.',
  canSpawn: false,
  enabled: true,
}

/** What the backend answers to each request, by method and path. */
function answer(method: string, path: string): Response {
  if (path.startsWith('/api/models')) {
    return Response.json({ providers: [] })
  }
  if (method === 'POST' && path === '/api/subagents/profiles') {
    const body = sent.at(-1)?.body
    const name = body !== null && typeof body === 'object' && 'name' in body ? body.name : null
    if (name === 'default') {
      return Response.json(
        { code: 'invalid_body', message: 'name: "default" is reserved for inheriting the parent' },
        { status: 400 },
      )
    }
    return Response.json({ profile: explore }, { status: 201 })
  }
  if (method === 'PATCH' && path === '/api/subagents/profiles/p-explore') {
    return Response.json({ profile: { ...explore, enabled: false } })
  }
  if (method === 'DELETE' || (method === 'PUT' && path === '/api/subagents')) {
    return new Response(null, { status: 204 })
  }
  throw new Error(`Unexpected request: ${method} ${path}`)
}

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  sent = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    const method = init?.method ?? 'GET'
    sent.push({ method, path, body: init?.body ? JSON.parse(String(init.body)) : null })
    return answer(method, path)
  }) as typeof fetch
  channels = playChannels()
  useProduct().start()
  channels.last().connect(productState({ subagents: { enabled: true, profiles: [reviewer] } }))
})

afterEach(() => {
  globalThis.fetch = realFetch
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
})

test('each write of the subagent settings shows at once as its answer left them, in name order', async () => {
  const subagents = useSubagentSettings()
  const product = useProduct()
  const names = () => product.snapshot?.subagents.profiles.map((profile) => profile.name)

  await subagents.saveProfile(null, {
    name: 'explore',
    description: 'Finds code.',
    model: { providerId: 'work', modelId: 'haiku', thinkingEffort: null, serviceTierId: null },
    instructions: 'You only report.',
    canSpawn: false,
  })
  expect(names()).toEqual(['explore', 'reviewer'])
  expect(sent.at(-1)?.body).toEqual({
    name: 'explore',
    description: 'Finds code.',
    model: { providerId: 'work', modelId: 'haiku', thinkingEffort: null, serviceTierId: null },
    instructions: 'You only report.',
    canSpawn: false,
  })

  await subagents.switchProfile('p-explore', false)
  expect(sent.at(-1)).toEqual({
    method: 'PATCH',
    path: '/api/subagents/profiles/p-explore',
    body: { enabled: false },
  })
  expect(product.snapshot?.subagents.profiles[0]?.enabled).toBe(false)

  await subagents.deleteProfile('p-explore')
  expect(names()).toEqual(['reviewer'])

  await subagents.switchSubagents(false)
  expect(sent.at(-1)).toEqual({ method: 'PUT', path: '/api/subagents', body: { enabled: false } })
  expect(product.snapshot?.subagents.enabled).toBe(false)

  // A refusal reaches the editor and changes nothing.
  const refused = subagents.saveProfile(null, {
    name: 'default',
    description: 'Inherits.',
    model: null,
    instructions: null,
    canSpawn: true,
  })
  await expect(refused).rejects.toThrow('name: "default" is reserved for inheriting the parent')
  expect(names()).toEqual(['reviewer'])
})
