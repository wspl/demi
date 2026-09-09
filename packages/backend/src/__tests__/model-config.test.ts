import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { AgentClient, createWebSocketClientTransport } from '@demicodes/agent'
import { defineProvider, type InferenceRequest } from '@demicodes/provider'
import { events } from '@demicodes/provider/testing'
import { openBackend } from './session'

const model = {
  id: 'configured',
  displayName: 'Configured model',
  contextWindow: 64000,
  outputLimit: 4000,
  thinkingEfforts: ['low', 'high'],
  acceptedExtensions: ['pdf'],
  fastTier: 'priority'
}
const json = (body: unknown, method = 'POST') => ({
  method,
  body: JSON.stringify(body),
  headers: { 'content-type': 'application/json' }
})

test(
  'manual model parameters reach inference and refresh preserves overrides with failure metadata',
  async () => {
    const requests: InferenceRequest[] = []
    const refreshed: boolean[] = []
    const backend = await openBackend({
      dataDir: await mkdtemp(join(tmpdir(), 'demi-model-config-')),
      port: 0,
      providerTypes: {
        configured: {
          credential: 'api_key',
          create: ({ providerId, label }) => defineProvider({
            id: providerId,
            displayName: label,
            listModels: async options => {
              refreshed.push(options?.refresh ?? false)
              throw new Error('Catalog unavailable')
            },
            createRuntime: () => {
              const runtime = {
                async *run(request: InferenceRequest) {
                  requests.push(request)
                  yield events.text('fixture')
                  yield events.response()
                },
                clone: () => runtime,
              }
              return runtime
            },
          })
        },
      }
    })
    let client: AgentClient | undefined
    try {
      const create = await backend.session.fetch('/api/providers', json({
        providerType: 'configured',
        label: 'Configured',
        apiKey: 'secret-key',
        models: [model]
      }))
      const { provider } = await create.json() as { provider: { id: string } }
      expect(create.status).toBe(201)
      expect(
        (await backend.session.fetch(
          `/api/providers/${provider.id}`,
          json({ models: [model, model] }, 'PATCH')
        )).status
      )
        .toBe(400)
      const catalog = await (await backend.session.fetch(
        '/api/models?refresh=true'
      )).json() as { providers: Array<{
          models: unknown[];
          stale: boolean;
          warnings: string[]
        }> }
      expect(refreshed).toEqual([true])
      expect(catalog.providers[0]).toMatchObject({
        stale: true,
        warnings: ['Catalog unavailable'],
        models: [expect.objectContaining({
            contextWindow: 64000,
            outputLimit: 4000,
            acceptedExtensions: ['pdf']
          })]
      })
      const { conversation } = await (await backend.session.fetch(
        '/api/conversations',
        { method: 'POST' }
      )).json() as { conversation: { id: string } }
      const socket = backend.session.socket(
        `/api/conversations/${conversation.id}/stream`
      )
      await new Promise<void>(
        resolve => socket.addEventListener(
          'open',
          () => resolve(),
          { once: true }
        )
      )
      client = new AgentClient(createWebSocketClientTransport(socket as never))
      await client.open({
        providerId: provider.id,
        model: {
          providerId: provider.id,
          model: {
            id: model.id,
            name: 'stale',
            contextWindow: 1000,
            outputLimit: 100,
            inputLimit: null,
            thinking: [],
            acceptedExtensions: []
          },
          thinking: { type: 'effort', effort: 'high', summary: null },
          serviceTierId: 'priority',
        }
      }, '/ignored', 'ignored')
      await client.send([{ type: 'text', text: 'first' }])
      expect(requests[0]).toMatchObject({
        modelId: model.id,
        outputLimit: 4000,
        serviceTierId: 'priority',
        thinking: { effort: 'high' }
      })
      await backend.session.fetch(
        `/api/providers/${provider.id}`,
        json({ models: [{ ...model, outputLimit: 8000 }] }, 'PATCH')
      )
      await client.send([{ type: 'text', text: 'second' }])
      expect(requests[1]?.outputLimit).toBe(8000)
      const status = await backend.session.fetch(
        `/api/providers/${provider.id}/status`
      )
      expect(await status.json())
        .toMatchObject({ quota: null, auth: { status: 'unknown' } })
      expect(
        await (await backend.session.fetch(
          `/api/providers/${provider.id}/quota`,
          { method: 'POST' }
        )).json()
      )
        .toEqual({ quota: null })
    } finally {
      await client?.close()
      await backend.close()
    }
  }
)
