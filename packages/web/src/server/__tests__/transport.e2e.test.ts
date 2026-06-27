import { expect, test } from 'bun:test'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { defineProvider, type Provider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { agentSocketUrl, connectAgentClient } from '@demicodes/web-ui/transport/agent-socket'
import { connectControlClient } from '@demicodes/web-ui/transport/control-client'
import { WEB_BACKEND_BASE_URL, WEB_BACKEND_PORT, WEB_FRONTEND_URL } from '../../dev-ports'
import { parseServerOptions } from '../server-options'
import { createWebProviders } from '../providers'
import { startWebServer } from '../serve'
import { createStubProvider } from '../stub-provider'

test('web dev ports are fixed and not configurable through startup options', () => {
  const options = parseServerOptions([])

  expect(options.port).toBe(WEB_BACKEND_PORT)
  expect(WEB_BACKEND_BASE_URL).toBe('http://127.0.0.1:18911')
  expect(WEB_FRONTEND_URL).toBe('http://127.0.0.1:18922')
  expect(() => parseServerOptions(['--port', '1234'])).toThrow('Unknown option: --port')
})

test('web transport round-trips open/send/stream over websocket', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-transport-'))
  const handle = startWebServer([createStubProvider()], testServerOptions(['--cwd', cwd]))

  try {
    const control = await connectControlClient(`${handle.url.replace(/^http/, 'ws')}/control`)

    const providers = await control.listProviders()
    expect(providers.map((provider) => provider.id)).toContain('claude-code')

    const models = await control.listModels({ providerId: 'claude-code' })
    expect(models[0]?.id).toBe('stub-model')

    const providerSelection = await control.prepareSession({ providerId: 'claude-code', modelId: 'stub-model' })
    expect(providerSelection).not.toHaveProperty('config')
    expect(providerSelection.model.model.id).toBe('stub-model')

    const client = await connectAgentClient(agentSocketUrl(handle.url, cwd))
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
  const handle = startWebServer([secretProvider], testServerOptions(['--cwd', cwd]))

  try {
    const control = await connectControlClient(`${handle.url.replace(/^http/, 'ws')}/control`)

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

test('web control protocol replaces selected provider catalog with explicit startup model', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-explicit-model-'))
  const options = testServerOptions([
    '--cwd',
    cwd,
    '--provider',
    'openai',
    '--model',
    'deepseek-v4-pro',
    '--model-context-window',
    '1000000',
    '--model-display-name',
    'DeepSeek V4 Pro',
    '--model-thinking-efforts',
    'low,medium,high,xhigh,max',
    '--model-can-disable-thinking',
    'false',
    '--thinking',
    'medium',
  ])
  const handle = startWebServer(createWebProviders(options), options)

  try {
    const control = await connectControlClient(`${handle.url.replace(/^http/, 'ws')}/control`)

    const models = await control.listModels({ providerId: 'openai' })
    expect(models.map((model) => model.id)).toEqual(['deepseek-v4-pro'])
    expect(models.map((model) => model.name)).toEqual(['DeepSeek V4 Pro'])
    expect(models[0]?.contextWindow).toBe(1_000_000)
    expect(models[0]?.reasoning).toEqual({
      efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
      defaultEffort: 'medium',
      canDisable: false,
    })

    const providerSelection = await control.prepareSession({ providerId: 'openai', modelId: 'deepseek-v4-pro' })
    expect(providerSelection.model.model.id).toBe('deepseek-v4-pro')
    expect(providerSelection.model.model.name).toBe('DeepSeek V4 Pro')
    expect(providerSelection.model.model.contextWindow).toBe(1_000_000)
    expect(providerSelection.model.thinking).toEqual({ type: 'effort', effort: 'medium', summary: null })
    expect(providerSelection.model.model.thinking).toEqual([
      {
        type: 'effort',
        efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
        defaultEffort: 'medium',
        summaries: ['auto', 'concise', 'detailed', 'off', 'on'],
        defaultSummary: null,
      },
    ])
  } finally {
    await handle.stop()
  }
})

test('web startup requires context window for explicit startup model', () => {
  expect(() =>
    parseServerOptions([
      '--provider',
      'openai',
      '--model',
      'deepseek-v4-pro',
    ]),
  ).toThrow('--model-context-window is required when --model is set')
})

test('web startup rejects explicit thinking default outside explicit model efforts', () => {
  expect(() =>
    parseServerOptions([
      '--provider',
      'openai',
      '--model',
      'deepseek-v4-pro',
      '--model-context-window',
      '1000000',
      '--model-thinking-efforts',
      'low,medium,high',
      '--thinking',
      'max',
    ]),
  ).toThrow('--thinking must be one of --model-thinking-efforts')
})

test('web backend rejects ordinary HTTP so UI must come from Vite dev server', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-backend-only-'))
  const handle = startWebServer([createStubProvider()], testServerOptions(['--cwd', cwd]))

  try {
    const response = await fetch(`${handle.url}/`)

    expect(response.status).toBe(404)
    expect(await response.text()).toContain('Open the Vite dev server')
  } finally {
    await handle.stop()
  }
})

function testServerOptions(args: string[]) {
  return { ...parseServerOptions(args), port: 0 }
}

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
