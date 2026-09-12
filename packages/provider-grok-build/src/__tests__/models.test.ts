import { expect, test } from 'bun:test'
import { GrokAuthError, type GrokAuthStore, type GrokResolvedAuth } from '../auth'
import { listGrokBuildModels, modelListFromGrokModelsPayload } from '../models'

const auth: GrokResolvedAuth = {
  accessToken: 'synthetic-token', refreshToken: null, expiresAt: null,
  email: null, userId: null, principalId: null, principalType: null,
  issuer: null, clientId: null, entryKey: 'synthetic', authFile: '/unused',
}
const store: GrokAuthStore = {
  resolveAuth: async () => auth,
  status: async () => ({ status: 'authenticated' }),
}

test('Grok catalog preserves explicit capabilities and unknown metadata without inventing defaults', () => {
  const result = modelListFromGrokModelsPayload({ data: [
    { id: 'unknown' },
    { id: 'no-reasoning', supports_reasoning_effort: false },
    { id: 'reasoning', context_window: 300000, input_modalities: ['text', 'image'],
      reasoning_efforts: [{ id: 'low' }, { id: 'high', default: true }] },
  ] }, 'custom')
  expect(result.models[0]).toMatchObject({
    providerId: 'custom', contextWindow: null, supportsTools: null,
    supportsAttachments: null, supportsReasoning: null, defaultThinkingEffort: null,
  })
  expect(result.models[1]?.supportsReasoning).toBe(false)
  expect(result.models[2]).toMatchObject({
    contextWindow: 300000, supportsAttachments: true, supportsReasoning: true,
    supportedThinkingEfforts: ['low', 'high'], defaultThinkingEffort: 'high',
  })
  expect(modelListFromGrokModelsPayload({ data: [] }, 'custom'))
    .toMatchObject({ models: [], defaultModelId: null, stale: false, warnings: [] })
  expect(modelListFromGrokModelsPayload({ data: [
    { id: 'm', reasoning_efforts: [{ id: 'high' }] },
  ] }, 'custom').models[0]?.defaultThinkingEffort).toBeNull()
})

test('Grok catalog rejects malformed entries instead of skipping them or returning defaults', () => {
  for (const value of [
    null, [], {}, { data: {} }, { data: [null] }, { data: [{}] },
    { data: [{ id: 1 }] }, { data: [{ id: 'm', context_window: '100' }] },
    { data: [{ id: 'm', context_window: 1.5 }] }, { data: [{ id: 'm', context_window: 0 }] },
    { data: [{ id: 'm', name: [] }] }, { data: [{ id: 'm', supports_reasoning_effort: 'true' }] },
    { data: [{ id: 'm', input_modalities: 'image' }] },
    { data: [{ id: 'm', reasoning_efforts: ['high'] }] },
    { data: [{ id: 'm', reasoning_efforts: [{ id: 'high', default: 1 }] }] },
    { data: [{ id: 'm', reasoning_efforts: [{ id: 'high' }], reasoning_effort: 'low' }] },
    { data: [{ id: 'm', reasoning_efforts: [{ id: 'high' }, { id: 'high' }] }] },
    { data: [{ id: 'm' }, { id: 'm' }] },
    { data: [{ id: 'm', supports_reasoning_effort: false, reasoning_effort: 'high' }] },
  ]) {
    expect(() => modelListFromGrokModelsPayload(value, 'grok'))
      .toThrow()
  }
})

test('Grok catalog reports malformed success responses and authorization errors directly', async () => {
  for (const response of [
    Response.json({ data: [{ id: 'm', context_window: 'private-value' }] }),
    new Response('{"private-value":'), new Response(null, { status: 401 }),
    new Response(null, { status: 403 }),
  ]) {
    await expect(listGrokBuildModels({ authStore: store, fetch: async () => response })).rejects.toThrow()
  }
  await expect(listGrokBuildModels({ authStore: {
    ...store, resolveAuth: async () => { throw new GrokAuthError('auth_invalid', 'invalid auth') },
  } })).rejects.toThrow('invalid auth')
})

test('Grok catalog fallbacks state why live discovery was unavailable', async () => {
  const missing = await listGrokBuildModels({ authStore: {
    ...store, resolveAuth: async () => { throw new GrokAuthError('auth_missing', 'missing') },
  } })
  expect(missing).toMatchObject({ stale: true })
  expect(missing.warnings[0]).toContain('credentials are missing')
  const offline = await listGrokBuildModels({ authStore: store, fetch: async () => { throw new Error('offline') } })
  expect(offline.stale).toBe(true)
  expect(offline.warnings[0]).toContain('network request failed')
  let cancelled = false
  const unavailable = await listGrokBuildModels({ authStore: store, fetch: async () =>
    new Response(new ReadableStream({ cancel() { cancelled = true } }), { status: 503 }) })
  expect(unavailable.warnings[0]).toContain('HTTP 503')
  expect(cancelled).toBe(true)
  const empty = await listGrokBuildModels({ authStore: store, fetch: async () => Response.json({ data: [] }) })
  expect(empty.models).toEqual([])
  expect(empty.stale).toBe(false)
})
