import { expect, test } from 'bun:test'
import {
  StaticCodexAuthStore,
  codexBackendModelsToModelList,
  listCodexModels,
  resetCodexModelCatalogCacheForTests,
  type CodexResolvedAuth,
} from '../index'

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

test('Codex backend model catalog maps slug ids and explicit capabilities', () => {
  const list = codexBackendModelsToModelList(codexModelsFixture(), {
    sourceFetchedAt: '2026-06-20T00:00:00.000Z',
  })

  expect(list.providerId).toBe('codex')
  expect(list.defaultModelId).toBeNull()
  expect(list.models.map((model) => model.id)).toEqual(['gpt-5.5', 'gpt-5.4-mini'])
  expect(list.models[0]).toMatchObject({
    providerId: 'codex',
    id: 'gpt-5.5',
    displayName: 'GPT-5.5',
    contextWindow: 272_000,
    outputLimit: null,
    supportsTools: true,
    supportsAttachments: true,
    supportsReasoning: true,
    supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh'],
    defaultThinkingEffort: 'medium',
    source: 'codex-backend',
    sourceFetchedAt: '2026-06-20T00:00:00.000Z',
    stale: false,
  })
})

test('listCodexModels requests Codex backend with auth headers and client version', async () => {
  resetCodexModelCatalogCacheForTests()
  const requests: Array<{ url: string; headers: Headers }> = []
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
  expect(requests[0]?.url).toBe('https://chatgpt.com/backend-api/codex/models?client_version=0.130.0')
  expect(requests[0]?.headers.get('authorization')).toBe('Bearer access-token')
  expect(requests[0]?.headers.get('chatgpt-account-id')).toBe('account-1')
  expect(requests[0]?.headers.get('accept')).toBe('application/json')
  expect(list.models.map((model) => model.id)).toEqual(['gpt-5.5', 'gpt-5.4-mini'])
  resetCodexModelCatalogCacheForTests()
})

test('listCodexModels refreshes auth once after a 401 catalog response', async () => {
  resetCodexModelCatalogCacheForTests()
  const refreshedAuth: CodexResolvedAuth = { ...chatgptAuth, accessToken: 'new-access-token' }
  const authStore = new RecordingAuthStore([chatgptAuth, refreshedAuth])
  const authorizationHeaders: string[] = []
  let calls = 0

  const list = await listCodexModels({
    authStore,
    clientVersion: '0.130.0',
    fetch: async (_url, init) => {
      calls += 1
      authorizationHeaders.push(new Headers(init?.headers).get('authorization') ?? '')
      if (calls === 1) return new Response('unauthorized', { status: 401 })
      return Response.json(codexModelsFixture())
    },
    now: () => new Date('2026-06-20T00:00:00.000Z'),
  })

  expect(authStore.forceRefreshes).toEqual([false, true])
  expect(authorizationHeaders).toEqual(['Bearer access-token', 'Bearer new-access-token'])
  expect(list.models.map((model) => model.id)).toEqual(['gpt-5.5', 'gpt-5.4-mini'])
  resetCodexModelCatalogCacheForTests()
})

test('listCodexModels rejects OPENAI_API_KEY auth for Codex backend catalog', async () => {
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
})

test('listCodexModels returns stale cache on non-auth catalog failures', async () => {
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
  expect(stale.models.every((model) => model.source === 'cache' && model.stale)).toBe(true)
  expect(stale.warnings.join('\n')).toContain('HTTP 503')
  expect(stale.models.map((model) => model.id)).toEqual(first.models.map((model) => model.id))
  resetCodexModelCatalogCacheForTests()
})

function codexModelsFixture(): unknown {
  return {
    models: [
      {
        slug: 'gpt-5.5',
        display_name: 'GPT-5.5',
        context_window: 272_000,
        input_modalities: ['text', 'image'],
        tool_mode: 'default',
        default_reasoning_level: 'medium',
        supported_reasoning_levels: [
          { effort: 'low' },
          { effort: 'medium' },
          { effort: 'high' },
          { effort: 'xhigh' },
        ],
      },
      {
        slug: 'gpt-5.4-mini',
        display_name: 'GPT-5.4-Mini',
        context_window: 272_000,
        input_modalities: ['text'],
        supported_reasoning_levels: [],
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

  async resolveAuth(options: { forceRefresh?: boolean } = {}): Promise<CodexResolvedAuth> {
    this.forceRefreshes.push(options.forceRefresh === true)
    const auth = this.auths[this.index] ?? this.auths[this.auths.length - 1]
    this.index += 1
    if (!auth) throw new Error('No fake auth left')
    return auth
  }
}
