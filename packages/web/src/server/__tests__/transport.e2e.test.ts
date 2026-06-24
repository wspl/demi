import { expect, test } from 'bun:test'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { defineProvider, type Provider } from '@demi/provider'
import { StubProvider, events } from '@demi/provider/testing'
import { agentSocketUrl, connectAgentClient } from '@demi/web-ui/transport/agent-socket'
import { connectControlClient } from '@demi/web-ui/transport/control-client'
import { parseServerOptions } from '../server-options'
import { startWebServer } from '../serve'
import { createStubProvider } from '../stub-provider'

test('web transport round-trips open/send/stream over websocket', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-transport-'))
  const handle = startWebServer([createStubProvider()], parseServerOptions(['--port', '0', '--cwd', cwd]))

  try {
    const control = await connectControlClient(`ws://localhost:${handle.port}/control`)

    const providers = await control.listProviders()
    expect(providers.map((provider) => provider.id)).toContain('claude-code')

    const models = await control.listModels({ providerId: 'claude-code' })
    expect(models[0]?.id).toBe('stub-model')

    const providerSelection = await control.prepareSession({ providerId: 'claude-code', modelId: 'stub-model' })
    expect(providerSelection).not.toHaveProperty('config')
    expect(providerSelection.model.model.id).toBe('stub-model')

    const client = await connectAgentClient(agentSocketUrl(`http://localhost:${handle.port}`, cwd))
    const seenText: string[] = []
    client.subscribe((event) => {
      if (event.type !== 'transcript_snapshot' && event.type !== 'transcript_patch') return
      for (const block of event.blocks) {
        if (block.type === 'text') seenText.push(block.text)
      }
    })

    await client.open(providerSelection, cwd)
    await client.send([{ type: 'text', text: 'hi' }])

    expect(seenText.some((text) => text.includes('Hello from the stub provider.'))).toBe(true)

    await client.close()
  } finally {
    await handle.stop()
  }
})

test('web control protocol does not expose secret-bearing provider options', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-secret-boundary-'))
  const secretProvider = createSecretBackedProvider()
  const handle = startWebServer([secretProvider], parseServerOptions(['--port', '0', '--cwd', cwd]))

  try {
    const control = await connectControlClient(`ws://localhost:${handle.port}/control`)

    const providers = await control.listProviders()
    const models = await control.listModels({ providerId: 'secret-api' })
    const providerSelection = await control.prepareSession({ providerId: 'secret-api', modelId: 'secret-model' })
    const browserVisibleJson = JSON.stringify({ providers, models, providerSelection })

    expect(browserVisibleJson).toContain('secret-api')
    expect(browserVisibleJson).toContain('secret-model')
    expect(browserVisibleJson).not.toContain('sk-secret')
    expect(browserVisibleJson).not.toContain('x-secret-token')
    expect(browserVisibleJson).not.toContain('https://secret-gateway.example/v1')
    expect(browserVisibleJson).not.toContain('SECRET_PREFIX')
    expect(providerSelection).not.toHaveProperty('config')
    expect(providerSelection).not.toHaveProperty('baseUrl')
    expect(providerSelection).not.toHaveProperty('envPrefix')
  } finally {
    await handle.stop()
  }
})

test('web control protocol prepends explicit startup model for selected provider', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-explicit-model-'))
  const provider = createCatalogProvider('openai', 'catalog-model')
  const handle = startWebServer(
    [provider],
    parseServerOptions(['--port', '0', '--cwd', cwd, '--provider', 'openai', '--model', 'deepseek-v4-pro']),
  )

  try {
    const control = await connectControlClient(`ws://localhost:${handle.port}/control`)

    const models = await control.listModels({ providerId: 'openai' })
    expect(models.map((model) => model.id).slice(0, 2)).toEqual(['deepseek-v4-pro', 'catalog-model'])

    const providerSelection = await control.prepareSession({ providerId: 'openai', modelId: 'deepseek-v4-pro' })
    expect(providerSelection.model.model.id).toBe('deepseek-v4-pro')
  } finally {
    await handle.stop()
  }
})

test('web backend rejects ordinary HTTP so UI must come from Vite dev server', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-backend-only-'))
  const handle = startWebServer([createStubProvider()], parseServerOptions(['--port', '0', '--cwd', cwd]))

  try {
    const response = await fetch(`${handle.url}/`)

    expect(response.status).toBe(404)
    expect(await response.text()).toContain('Open the Vite dev server')
  } finally {
    await handle.stop()
  }
})

function createSecretBackedProvider(): Provider {
  const secret = {
    baseUrl: 'https://secret-gateway.example/v1',
    apiKey: 'sk-secret',
    envPrefix: 'SECRET_PREFIX',
    headers: { 'x-secret-token': 'x-secret-token' },
  }

  return defineProvider({
    id: 'secret-api',
    displayName: 'Secret API',
    state: () => ({ status: 'ready' }),
    listModels: () => ({
      providerId: 'secret-api',
      defaultModelId: 'secret-model',
      warnings: [],
      sourceFetchedAt: '1970-01-01T00:00:00.000Z',
      stale: false,
      models: [
        {
          providerId: 'secret-api',
          id: 'secret-model',
          displayName: 'Secret Model',
          contextWindow: 1000,
          outputLimit: 1000,
          supportsTools: true,
          supportsAttachments: false,
          supportsReasoning: false,
          supportedThinkingEfforts: null,
          defaultThinkingEffort: null,
          sourceFetchedAt: '1970-01-01T00:00:00.000Z',
          stale: false,
        },
      ],
    }),
    createRuntime: () => {
      void secret
      return new StubProvider([[events.text('secret ok'), events.response()]])
    },
  })
}

function createCatalogProvider(providerId: string, modelId: string): Provider {
  return defineProvider({
    id: providerId,
    displayName: providerId,
    state: () => ({ status: 'ready' }),
    listModels: () => ({
      providerId,
      defaultModelId: modelId,
      warnings: [],
      sourceFetchedAt: '1970-01-01T00:00:00.000Z',
      stale: false,
      models: [
        {
          providerId,
          id: modelId,
          displayName: modelId,
          contextWindow: 1000,
          outputLimit: 1000,
          supportsTools: true,
          supportsAttachments: false,
          supportsReasoning: false,
          supportedThinkingEfforts: null,
          defaultThinkingEffort: null,
          sourceFetchedAt: '1970-01-01T00:00:00.000Z',
          stale: false,
        },
      ],
    }),
    createRuntime: () => new StubProvider([[events.text('catalog ok'), events.response()]]),
  })
}
