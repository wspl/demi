import { createCodexProvider } from '../provider'
import { expect, spyOn, test } from 'bun:test'
import { StaticCodexAuthStore, type CodexResolvedAuth } from '../auth'
import {
  codexBackendModelsToModelList,
  listCodexModels,
  resetCodexModelCatalogCacheForTests,
} from '../models'

const chatgptAuth: CodexResolvedAuth = {
  kind: 'chatgpt',
  mode: 'chatgpt',
  accessToken: 'access-token',
  refreshToken: 'refresh-token',
  accountId: 'account-1',
  email: 'dev@example.com',
  isFedrampAccount: false,
  expiresAt: null,
  authFile: '/tmp/auth.json',
}

test(
  'Provider.listModels forwards refresh and retains the configured Codex model filter',
  async () => {
    resetCodexModelCatalogCacheForTests()
    const fetch = spyOn(globalThis, 'fetch').mockImplementation(Object.assign(
      async () => Response.json(codexModelsFixture()),
      { preconnect() {} }
    ))
    try {
      const provider = createCodexProvider({
        authStore: new StaticCodexAuthStore(chatgptAuth),
        models: { include: ['gpt-5.5'] }
      })
      await provider.listModels!()
      await provider.listModels!()
      expect(fetch).toHaveBeenCalledTimes(1)
      const refreshed = await provider.listModels!({ refresh: true })
      expect(fetch).toHaveBeenCalledTimes(2)
      expect(refreshed.models.map((model) => model.id)).toEqual(['gpt-5.5'])
    } finally {
      fetch.mockRestore()
      resetCodexModelCatalogCacheForTests()
    }
  }
)

test(
  'Codex backend model catalog maps slug ids and explicit capabilities',
  () => {
    const list = codexBackendModelsToModelList(codexModelsFixture(), {
      sourceFetchedAt: '2026-06-20T00:00:00.000Z',
    })

    expect(list.providerId).toBe('codex')
    expect(list.defaultModelId).toBe('gpt-5.5')
    expect(list.models.map((model) => model.id)).toEqual([
      'gpt-5.5',
      'gpt-5.4-mini'
    ])
    expect(list.models[0]).toMatchObject({
      providerId: 'codex',
      id: 'gpt-5.5',
      displayName: 'GPT-5.5',
      contextWindow: 272_000,
      outputLimit: null,
      supportsTools: true,
      supportsAttachments: true,
      supportsReasoning: true,
      canDisableThinking: false,
      supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'ultra'],
      defaultThinkingEffort: 'medium',
      serviceTiers: [{
        id: 'priority',
        label: 'Fast',
        description: '1.5x speed, increased usage',
        fast: true
      }],
      defaultServiceTierId: null,
      sourceFetchedAt: '2026-06-20T00:00:00.000Z',
      stale: false,
    })
  }
)

test('Codex catalog keeps full efforts and defaults in priority order', () => {
  const template = codexModelsFixture().models[0]!
  const list = codexBackendModelsToModelList({
    models: [
      { ...template, slug: 'later', priority: 20, tool_mode: null },
      {
        ...template,
        slug: 'first',
        priority: 1,
        supported_reasoning_levels: [
          { effort: 'low' },
          { effort: 'medium' },
          { effort: 'high' },
          { effort: 'xhigh' },
          { effort: 'max' },
          { effort: 'ultra' },
        ],
        default_reasoning_level: 'low',
        default_service_tier: 'priority',
      },
      { ...template, slug: 'hidden', visibility: 'hide', priority: 0 },
      { ...template, slug: 'internal', visibility: 'none', priority: 0 },
    ],
  })
  expect(list.models.map((model) => model.id)).toEqual(['first', 'later'])
  expect(list.defaultModelId).toBe('first')
  expect(list.models[0]).toMatchObject({
    supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'],
    defaultThinkingEffort: 'low',
    defaultServiceTierId: 'priority',
  })
})

test('Codex catalog rejects malformed metadata instead of dropping entries', () => {
  const model = codexModelsFixture().models[0]!
  expect(() => codexBackendModelsToModelList({ models: [
    model,
    { ...model, supported_reasoning_levels: [{ effort: 3 }] },
  ] })).toThrow()
  expect(() => codexBackendModelsToModelList({ models: [
    { ...model, default_reasoning_level: 3 },
  ] })).toThrow()
})

test(
  'listCodexModels requests Codex backend with auth headers and client version',
  async () => {
    resetCodexModelCatalogCacheForTests()
    const requests: Array<{
      url: string;
      headers: Headers
    }> = []
    const list = await listCodexModels({
      authStore: new StaticCodexAuthStore(chatgptAuth),
      clientVersion: '0.130.0',
      baseUrl: 'https://chatgpt.com/backend-api',
      fetch: async (url, init) => {
        requests.push({ url: String(url), headers: new Headers(init?.headers) })
        return Response.json(codexModelsFixture())
      },
      now: () => new Date('2026-06-20T00:00:00.000Z'),
    })

    expect(requests).toHaveLength(1)
    expect(requests[0]?.url).toBe(
      'https://chatgpt.com/backend-api/codex/models?client_version=0.130.0'
    )
    expect(requests[0]?.headers.get('authorization'))
      .toBe('Bearer access-token')
    expect(requests[0]?.headers.get('chatgpt-account-id')).toBe('account-1')
    expect(requests[0]?.headers.get('accept')).toBe('application/json')
    expect(list.models.map((model) => model.id)).toEqual([
      'gpt-5.5',
      'gpt-5.4-mini'
    ])
    resetCodexModelCatalogCacheForTests()
  }
)

test(
  'an explicit catalog refresh bypasses the Codex TTL and reports failures as stale',
  async () => {
    resetCodexModelCatalogCacheForTests()
    let calls = 0
    let offline = false
    const options = {
      authStore: new StaticCodexAuthStore(chatgptAuth),
      now: () => new Date('2026-09-08T00:00:00Z'),
      fetch: async () => {
        calls += 1
        if (offline)
          throw new Error('offline')
        return Response.json(codexModelsFixture())
      },
    }
    await listCodexModels(options)
    await listCodexModels(options)
    expect(calls).toBe(1)
    await listCodexModels({ ...options, refresh: true })
    expect(calls).toBe(2)
    offline = true
    const stale = await listCodexModels({ ...options, refresh: true })
    expect(calls).toBe(3)
    expect(stale.stale).toBe(true)
    expect(stale.warnings.join(' ')).toContain('offline')
    resetCodexModelCatalogCacheForTests()
  }
)

test(
  'listCodexModels uses the static catalog client version by default',
  async () => {
    resetCodexModelCatalogCacheForTests()
    const requests: string[] = []
    await listCodexModels({
      authStore: new StaticCodexAuthStore(chatgptAuth),
      fetch: async (url) => {
        requests.push(String(url))
        return Response.json(codexModelsFixture())
      },
      now: () => new Date('2026-06-20T00:00:00.000Z'),
    })

    expect(requests).toEqual(
      ['https://chatgpt.com/backend-api/codex/models?client_version=0.153.4']
    )
    resetCodexModelCatalogCacheForTests()
  }
)

test(
  'listCodexModels refreshes auth once after a 401 catalog response',
  async () => {
    resetCodexModelCatalogCacheForTests()
    const refreshedAuth: CodexResolvedAuth = {
      ...chatgptAuth,
      accessToken: 'new-access-token'
    }
    const authStore = new RecordingAuthStore([chatgptAuth, refreshedAuth])
    const authorizationHeaders: string[] = []
    let calls = 0

    const list = await listCodexModels({
      authStore,
      clientVersion: '0.130.0',
      fetch: async (_url, init) => {
        calls += 1
        authorizationHeaders.push(
          new Headers(init?.headers).get('authorization')
            ?? ''
        )
        if (calls === 1)
          return new Response('unauthorized', { status: 401 })
        return Response.json(codexModelsFixture())
      },
      now: () => new Date('2026-06-20T00:00:00.000Z'),
    })

    expect(authStore.forceRefreshes).toEqual([false, true])
    expect(authorizationHeaders).toEqual([
      'Bearer access-token',
      'Bearer new-access-token'
    ])
    expect(list.models.map((model) => model.id)).toEqual([
      'gpt-5.5',
      'gpt-5.4-mini'
    ])
    resetCodexModelCatalogCacheForTests()
  }
)

test(
  'listCodexModels rejects OPENAI_API_KEY auth for Codex backend catalog',
  async () => {
    await expect(
      listCodexModels({
        authStore: new StaticCodexAuthStore({
          kind: 'apiKey',
          mode: 'apiKey',
          apiKey: 'sk-test',
          authFile: null,
        }),
        clientVersion: '0.130.0',
        fetch: async () => Response.json(codexModelsFixture()),
      }),
    ).rejects.toThrow('requires official Codex ChatGPT auth')
  }
)

test(
  'listCodexModels returns stale cache on non-auth catalog failures',
  async () => {
    resetCodexModelCatalogCacheForTests()
    const now = new Date('2026-06-20T00:00:00.000Z')
    const first = await listCodexModels({
      authStore: new StaticCodexAuthStore(chatgptAuth),
      clientVersion: '0.130.0',
      fetch: async () => Response.json(codexModelsFixture()),
      now: () => now,
    })

    const stale = await listCodexModels({
      authStore: new StaticCodexAuthStore(chatgptAuth),
      clientVersion: '0.130.0',
      fetch: async () => new Response('overloaded', { status: 503 }),
      now: () => new Date(now.getTime() + 16 * 60 * 1000),
    })

    expect(stale.stale).toBe(true)
    expect(stale.models.every((model) => model.stale)).toBe(true)
    expect(stale.warnings.join('\n')).toContain('HTTP 503')
    expect(stale.models.map((model) => model.id))
      .toEqual(first.models.map((model) => model.id))
    resetCodexModelCatalogCacheForTests()
  }
)

function codexModelsFixture() {
  return {
    models: [
      {
        slug: 'gpt-5.5',
        visibility: 'list',
        priority: 1,
        display_name: 'GPT-5.5',
        context_window: 272_000,
        input_modalities: ['text', 'image'],
        tool_mode: 'default',
        default_reasoning_level: 'medium',
        service_tiers: [
          {
            id: 'priority',
            name: 'Fast',
            description: '1.5x speed, increased usage'
          },
        ],
        additional_speed_tiers: ['fast'],
        supported_reasoning_levels: [
          { effort: 'low' },
          { effort: 'medium' },
          { effort: 'high' },
          { effort: 'xhigh' },
          { effort: 'ultra' },
        ],
      },
      {
        slug: 'gpt-5.4-mini',
        visibility: 'list',
        priority: 2,
        display_name: 'GPT-5.4-Mini',
        context_window: 272_000,
        input_modalities: ['text'],
        supported_reasoning_levels: [],
      },
      {
        slug: 'codex-auto-review',
        display_name: 'Codex Auto Review',
        description: 'Automatic approval review model for Codex.',
        context_window: 272_000,
        visibility: 'hide',
        priority: 0,
        input_modalities: ['text', 'image'],
        supported_reasoning_levels: [{ effort: 'medium' }],
      },
    ],
  }
}

class RecordingAuthStore {
  readonly forceRefreshes: boolean[] = []
  private index = 0

  constructor(private readonly auths: CodexResolvedAuth[]) {}

  async status() {
    return { status: 'authenticated' as const }
  }

  async resolveAuth(
    options: { forceRefresh?: boolean } = {}
  ): Promise<CodexResolvedAuth> {
    this.forceRefreshes.push(options.forceRefresh === true)
    const auth = this.auths[this.index] ?? this.auths[this.auths.length - 1]
    this.index += 1
    if (!auth)
      throw new Error('No fake auth left')
    return auth
  }
}

test('Codex null tool metadata remains unknown rather than advertising tool support', () => {
  const model = codexModelsFixture().models[0]!
  const result = codexBackendModelsToModelList({ models: [{ ...model,
    tool_mode: null, experimental_supported_tools: undefined,
    apply_patch_tool_type: null, web_search_tool_type: null,
  }] })
  expect(result.models[0]?.supportsTools).toBeNull()
})
