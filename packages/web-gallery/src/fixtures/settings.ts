import { reactive } from 'vue'
import type { SettingsMcpServer, SettingsMcpState, SettingsProviderAccount, SettingsProviderEntry, SettingsProviderModel, SettingsProviderState, SettingsQuotaWindow, SettingsSkillSource, SettingsVendor, SettingsWireApi } from '@demicodes/web-ui/settings/types'

/**
 * A coding agent's whole settings surface, mocked in every awkward state at once:
 * expiring auth, a crashed server, a full quota, a disabled provider.
 */
export type ServerState = SettingsMcpState

/** Wire protocols the openai family can speak; the others have one each. */
export type WireApi = SettingsWireApi

/** The shared model plus what the mock knows but the page does not show. */
export interface MockModel extends SettingsProviderModel {
  tools: boolean | null
  defaultEffort: string | null
}

export type QuotaWindow = SettingsQuotaWindow
export type MockAccount = SettingsProviderAccount
export type MockProviderState = SettingsProviderState

export interface MockProvider extends SettingsProviderEntry {
  /** Runtime family; subscriptions name their CLI vendor. */
  family: string
  models: MockModel[]
}

export type MockServer = SettingsMcpServer

function model(partial: Partial<MockModel> & Pick<MockModel, 'id'>): MockModel {
  return {
    name: '',
    contextWindow: null,
    outputLimit: null,
    tools: null,
    extensions: [],
    efforts: [],
    defaultEffort: null,
    fastTier: null,
    enabled: true,
    ...partial,
  }
}

export function provider(partial: Partial<MockProvider> & Pick<MockProvider, 'id' | 'name' | 'kind' | 'family'>): MockProvider {
  return {
    vendorId: null,
    baseUrl: '',
    wireApi: 'openai-chat',
    apiKey: '',
    modelSource: 'catalog',
    catalogFetched: null,
    stale: false,
    state: 'ready',
    enabled: true,
    models: [],
    accounts: [],
    logo: null,
    ...partial,
  }
}

/** models.dev vendors one of Demi's runtimes can speak to, with their marks. */
export interface MockVendor extends SettingsVendor {
  family: 'anthropic' | 'openai' | 'google'
}

export const mockVendors: MockVendor[] = [
  { id: 'anthropic', name: 'Anthropic', family: 'anthropic', wireApi: 'anthropic-messages', baseUrl: null, logo: '/logos/anthropic.svg' },
  { id: 'openai', name: 'OpenAI', family: 'openai', wireApi: 'openai-responses', baseUrl: null, logo: '/logos/openai.svg' },
  { id: 'google', name: 'Google', family: 'google', wireApi: 'openai-chat', baseUrl: null, logo: '/logos/google.svg' },
  { id: 'google-vertex', name: 'Vertex', family: 'google', wireApi: 'openai-chat', baseUrl: null, logo: '/logos/google-vertex.svg' },
  { id: 'deepseek', name: 'DeepSeek', family: 'openai', wireApi: 'openai-chat', baseUrl: 'https://api.deepseek.com', logo: '/logos/deepseek.svg' },
  { id: 'moonshotai', name: 'Moonshot AI', family: 'openai', wireApi: 'openai-chat', baseUrl: 'https://api.moonshot.ai/v1', logo: '/logos/moonshotai.svg' },
  { id: 'zhipuai', name: 'Zhipu AI', family: 'openai', wireApi: 'openai-chat', baseUrl: 'https://open.bigmodel.cn/api/paas/v4', logo: '/logos/zhipuai.svg' },
  { id: 'minimax', name: 'MiniMax', family: 'anthropic', wireApi: 'anthropic-messages', baseUrl: 'https://api.minimax.io/anthropic/v1', logo: '/logos/minimax.svg' },
  { id: 'fireworks-ai', name: 'Fireworks AI', family: 'openai', wireApi: 'openai-chat', baseUrl: 'https://api.fireworks.ai/inference/v1', logo: '/logos/fireworks-ai.svg' },
  { id: 'alibaba', name: 'Alibaba', family: 'openai', wireApi: 'openai-chat', baseUrl: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1', logo: '/logos/alibaba.svg' },
  { id: 'openrouter', name: 'OpenRouter', family: 'openai', wireApi: 'openai-chat', baseUrl: 'https://openrouter.ai/api/v1', logo: '/logos/openrouter.svg' },
]

export function mockProviders(): MockProvider[] {
  return [
    provider({
      id: 'claude-code', name: 'Claude Code', kind: 'subscription', family: 'claude-code', logo: '/logos/claude.svg',
      accounts: [
        { id: 'a1', label: 'zan@example.com', plan: 'Max 5×', active: true, quota: { hour: { used: 62, max: 100, resets: 'in 2 h 10 min' }, week: { used: 31, max: 100, resets: 'Monday' } } },
        { id: 'a2', label: 'zan@work.example', plan: 'Pro', active: false, quota: { hour: { used: 100, max: 100, resets: 'in 4 h' }, week: { used: 88, max: 100, resets: 'Thursday' } } },
      ],
      catalogFetched: '2 min ago',
      models: [
        model({ id: 'claude-opus-4-1', name: 'Claude Opus 4.1', contextWindow: 200_000, outputLimit: 32_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: ['low', 'medium', 'high', 'max'], defaultEffort: 'high' }),
        model({ id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', contextWindow: 1_000_000, outputLimit: 64_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: ['low', 'medium', 'high', 'max'], defaultEffort: 'medium', fastTier: 'fast' }),
        model({ id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', contextWindow: 200_000, outputLimit: 64_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: [], enabled: false }),
      ],
    }),
    provider({
      id: 'codex', name: 'Codex', kind: 'subscription', family: 'codex', state: 'signed-out', logo: '/logos/openai.svg',
      catalogFetched: null,
      models: [
        model({ id: 'gpt-5-codex', name: 'GPT-5 Codex', contextWindow: 400_000, outputLimit: 128_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' }),
      ],
    }),
    provider({ id: 'grok-build', name: 'Grok Build', kind: 'subscription', family: 'grok-build', state: 'signed-out', logo: '/logos/xai.svg' }),
    provider({
      id: 'anthropic', name: 'Anthropic', kind: 'api_key', family: 'anthropic', vendorId: 'anthropic', logo: '/logos/anthropic.svg',
      baseUrl: 'https://api.anthropic.com', wireApi: 'anthropic-messages', apiKey: 'sk-ant-api03-3f2a9c1d7e5b4a6f8c2d1e9b', testedIn: '412 ms',
      catalogFetched: '14 min ago',
      models: [
        model({ id: 'claude-opus-4-1', name: 'Claude Opus 4.1', contextWindow: 200_000, outputLimit: 32_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: ['low', 'medium', 'high'], defaultEffort: 'high' }),
        model({ id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', contextWindow: 1_000_000, outputLimit: 64_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' }),
        model({ id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', contextWindow: 200_000, outputLimit: 64_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: [] }),
        model({ id: 'claude-3-5-sonnet-20241022', name: 'Claude 3.5 Sonnet', contextWindow: 200_000, outputLimit: 8_192, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: [], enabled: false }),
      ],
    }),
    provider({
      id: 'openai', name: 'OpenAI', kind: 'api_key', family: 'openai', vendorId: 'openai', logo: '/logos/openai.svg',
      baseUrl: 'https://api.openai.com/v1', wireApi: 'openai-responses', apiKey: 'sk-proj-91ce4a7b2d8f6e1c3a5b9d7f',
      state: 'error', detail: '401 · Incorrect API key provided', catalogFetched: '3 days ago', stale: true,
      models: [
        model({ id: 'gpt-5', name: 'GPT-5', contextWindow: 400_000, outputLimit: 128_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: ['minimal', 'low', 'medium', 'high'], defaultEffort: 'medium', fastTier: 'priority' }),
        model({ id: 'gpt-5-mini', name: 'GPT-5 mini', contextWindow: 400_000, outputLimit: 128_000, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: ['minimal', 'low', 'medium', 'high'], defaultEffort: 'low' }),
      ],
    }),
    provider({
      id: 'kimi', name: 'Kimi', kind: 'api_key', family: 'anthropic', vendorId: 'moonshotai', logo: '/logos/moonshotai.svg',
      baseUrl: 'https://api.moonshot.cn/anthropic', wireApi: 'anthropic-messages', apiKey: 'sk-8c1d5e2f9a7b3c6d4e1f0a9b', testedIn: '412 ms',
      modelSource: 'manual',
      models: [
        model({ id: 'kimi-k2-thinking', name: 'Kimi K2 Thinking', contextWindow: 256_000, outputLimit: 32_000, tools: true, extensions: [], efforts: ['low', 'high'], defaultEffort: 'high' }),
        model({ id: 'kimi-k2-turbo-preview' }),
      ],
    }),
    provider({
      id: 'ollama', name: 'Ollama', kind: 'api_key', family: 'openai', vendorId: null,
      baseUrl: 'http://localhost:11434/v1', wireApi: 'openai-chat', apiKey: '',
      modelSource: 'manual', state: 'unreachable', detail: 'connect ECONNREFUSED 127.0.0.1:11434',
      models: [model({ id: 'qwen3:32b', contextWindow: 40_000, tools: true, extensions: [] })],
    }),
    provider({
      id: 'vertex', name: 'Google Vertex', kind: 'api_key', family: 'google', vendorId: 'google-vertex', logo: '/logos/google-vertex.svg',
      baseUrl: 'https://us-central1-aiplatform.googleapis.com', wireApi: 'openai-chat', apiKey: 'ya29.a0AfB_byC1d2E3f4G5h6', state: 'disabled', enabled: false,
      catalogFetched: '1 h ago',
      models: [model({ id: 'gemini-2.5-pro', name: 'Gemini 2.5 Pro', contextWindow: 1_000_000, outputLimit: 65_536, tools: true, extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'], efforts: ['low', 'high'] })],
    }),
  ]
}

export function createSettingsState() {
  return reactive({
    general: {
      language: 'English',
      theme: 'system' as 'light' | 'dark' | 'system',
      fontSize: 15,
    },
    notifications: {
      browser: true,
      sound: false,
      onFinish: true,
      onError: true,
    },
    account: {
      name: 'Zan',
      email: 'zan@example.com',
      passwordChanged: '3 months ago',
      plan: 'Pro',
    },
    providers: mockProviders(),
    selectedProviderId: 'kimi' as string | null,
    providerDetailOpen: false,
    servers: [
      {
        id: 'github', name: 'GitHub', transport: 'stdio', target: 'npx @modelcontextprotocol/server-github', state: 'connected', enabled: true,
        tools: [
          'create_issue',
          'list_issues',
          'create_pull_request',
          'list_pull_requests',
          'merge_pull_request',
          'search_code',
          'get_file',
          'create_comment',
        ],
      },
      { id: 'postgres', name: 'Postgres', transport: 'http', target: 'https://mcp.internal/pg', state: 'auth', enabled: true, detail: 'Sign in to authorize this server', tools: [] },
      { id: 'filesystem', name: 'FileSystem', transport: 'stdio', target: 'npx @modelcontextprotocol/server-filesystem', state: 'crashed', enabled: true, detail: 'exit code 1 · npx: command not found', tools: ['read_file', 'write_file', 'list_directory', 'search_files'] },
      { id: 'sentry', name: 'Sentry', transport: 'http', target: 'https://mcp.sentry.dev', state: 'disabled', enabled: false, tools: [] },
    ] as MockServer[],
    skillSources: [
      {
        id: 'vercel',
        name: 'vercel-labs/agent-skills',
        origin: 'https://github.com/vercel-labs/agent-skills',
        state: 'ready',
        skills: [
          { id: 'web-design', name: 'web-design-guidelines', description: 'Review UI against Vercel’s web interface guidelines.', enabled: true },
          { id: 'react-best', name: 'vercel-react-best-practices', description: 'React composition and data-fetching patterns.', enabled: true },
          { id: 'react-native', name: 'vercel-react-native-skills', description: 'React Native layout and navigation conventions.', enabled: false },
          { id: 'composition', name: 'vercel-composition-patterns', description: 'When to split a component and when to leave it.', enabled: true },
          { id: 'frontend', name: 'frontend-design', description: 'Taste-led interface work: type, color, motion.', enabled: false },
          { id: 'tdd', name: 'tdd', description: 'Write the failing test before the change.', enabled: false },
          { id: 'agent-browser', name: 'agent-browser', description: 'Browse and act on a page the agent can see.', enabled: false },
          { id: 'find-skills', name: 'find-skills', description: 'Search installed skills when the next step is unclear.', enabled: true },
        ],
      },
      {
        id: 'anthropic',
        name: 'anthropics/skills',
        origin: 'https://github.com/anthropics/skills',
        state: 'ready',
        skills: [
          { id: 'pptx', name: 'pptx', description: 'Create and edit PowerPoint decks.', enabled: true },
          { id: 'pdf', name: 'pdf', description: 'Read and fill PDF forms.', enabled: true },
          { id: 'xlsx', name: 'xlsx', description: 'Build spreadsheets from tables.', enabled: true },
          { id: 'docx', name: 'docx', description: 'Draft Word documents.', enabled: false },
          { id: 'skill-creator', name: 'skill-creator', description: 'Author a new SKILL.md that other agents can load.', enabled: true },
        ],
      },
      {
        id: 'commit',
        name: 'zan/commit',
        origin: 'https://github.com/zan/commit',
        state: 'ready',
        skills: [
          { id: 'commit', name: 'commit', description: 'Conventional commit from the staged diff.', enabled: true },
        ],
      },
      {
        id: 'broken',
        name: 'example/broken-skills',
        origin: 'https://github.com/example/broken-skills',
        state: 'error',
        detail: 'Repository not found',
        skills: [],
      },
    ] as SettingsSkillSource[],
    archived: [
      { id: 'oauth', title: 'OAuth refresh token rotation', detail: 'Archived Aug 2' },
      { id: 'icons', title: 'Icon theme survey', detail: 'Archived Jul 21' },
      { id: 'perf', title: 'Sidebar scroll performance', detail: 'Archived Jul 3' },
    ],
    devices: [
      { id: 'mac', name: 'zan-mbp', online: true, seen: 'Now' },
      { id: 'build', name: 'build-01', online: false, seen: '3 days ago' },
      { id: 'lab', name: 'lab-workstation-with-a-long-hostname', online: true, seen: '2 minutes ago' },
    ],
    keys: [
      { id: 'new', action: 'New conversation', keys: '⌘⇧O' },
      { id: 'send', action: 'Send message', keys: '⏎' },
      { id: 'stop', action: 'Stop the turn', keys: '⎋' },
      { id: 'sidebar', action: 'Toggle sidebar', keys: '⌘B' },
      { id: 'search', action: 'Search conversations', keys: '⌘K' },
      { id: 'focus', action: 'Focus composer', keys: '⌘J' },
      { id: 'settings', action: 'Open settings', keys: '⌘,' },
    ],
    data: {
      retention: 'Forever',
      shareLinks: false,
      telemetry: true,
    },
  })
}

export type SettingsState = ReturnType<typeof createSettingsState>
