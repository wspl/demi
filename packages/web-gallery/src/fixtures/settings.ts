import { reactive } from 'vue'
import {
  Bell,
  BookOpen,
  Bot,
  CircleUser,
  Code,
  Database,
  Gauge,
  Keyboard,
  Monitor,
  Plug,
  Settings2,
  ShieldCheck,
  Sparkles,
} from '@lucide/vue'
import type { SettingsNavGroup } from '@demicodes/web-ui/settings/types'

/**
 * A coding agent's whole settings surface, mocked in every awkward state at once:
 * expiring auth, a crashed server, a shortcut conflict, a full quota, a disabled provider.
 */
export const fullSettingsNav: SettingsNavGroup[] = [
  {
    items: [
      { id: 'general', label: 'General', icon: Settings2 },
      { id: 'account', label: 'Account', icon: CircleUser },
      { id: 'notifications', label: 'Notifications', icon: Bell },
    ],
  },
  {
    label: 'Agent',
    items: [
      { id: 'models', label: 'Models & providers', icon: Sparkles },
      { id: 'agents', label: 'Agents', icon: Bot },
      { id: 'permissions', label: 'Permissions', icon: ShieldCheck },
      { id: 'instructions', label: 'Instructions & memory', icon: BookOpen },
      { id: 'mcp', label: 'MCP servers', icon: Plug },
    ],
  },
  {
    label: 'Workspace',
    items: [
      { id: 'devices', label: 'Devices', icon: Monitor },
      { id: 'keyboard', label: 'Keyboard', icon: Keyboard },
      { id: 'data', label: 'Data & privacy', icon: Database },
      { id: 'usage', label: 'Usage & billing', icon: Gauge },
    ],
  },
  {
    label: 'Advanced',
    items: [{ id: 'developer', label: 'Developer', icon: Code }],
  },
]

export type Permission = 'allow' | 'ask' | 'deny'
export type ServerState = 'connected' | 'auth' | 'crashed' | 'disabled'

/** Wire protocols the openai family can speak; the others have one each. */
export type WireApi = 'anthropic-messages' | 'openai-responses' | 'openai-chat'

export interface MockModel {
  id: string
  /** Empty when the id is all we know. */
  name: string
  contextWindow: number | null
  outputLimit: number | null
  tools: boolean | null
  attachments: boolean | null
  efforts: string[]
  defaultEffort: string | null
  fastTier: string | null
  enabled: boolean
}

export interface MockAccount {
  id: string
  label: string
  plan: string
  active: boolean
  /** Rate-window quota, when the vendor exposes one. */
  quota: { used: number; max: number; resets: string } | null
}

export type MockProviderState = 'ready' | 'error' | 'unreachable' | 'signed-out' | 'disabled'

export interface MockProvider {
  id: string
  name: string
  kind: 'api_key' | 'subscription'
  /** Runtime family; subscriptions name their CLI vendor. */
  family: string
  /** models.dev vendor; null for a custom endpoint. */
  vendorId: string | null
  baseUrl: string
  wireApi: WireApi
  keyHint: string
  /** Where the model list comes from: the vendor catalog, or ids the user typed. */
  modelSource: 'catalog' | 'manual'
  catalogFetched: string | null
  stale: boolean
  state: MockProviderState
  detail?: string
  enabled: boolean
  defaultModelId: string | null
  models: MockModel[]
  accounts: MockAccount[]
  /** A device-code login in flight. */
  login: { url: string; code: string; expires: string } | null
}

export interface MockServer {
  id: string
  name: string
  transport: 'stdio' | 'http'
  target: string
  state: ServerState
  enabled: boolean
  detail?: string
  tools: { name: string; enabled: boolean }[]
  expanded: boolean
}

export interface MockAgent {
  id: string
  name: string
  model: string
  mode: 'primary' | 'subagent'
  summary: string
  isDefault?: boolean
  enabled: boolean
  expanded: boolean
}

function model(partial: Partial<MockModel> & Pick<MockModel, 'id'>): MockModel {
  return {
    name: '',
    contextWindow: null,
    outputLimit: null,
    tools: null,
    attachments: null,
    efforts: [],
    defaultEffort: null,
    fastTier: null,
    enabled: true,
    ...partial,
  }
}

function provider(partial: Partial<MockProvider> & Pick<MockProvider, 'id' | 'name' | 'kind' | 'family'>): MockProvider {
  return {
    vendorId: null,
    baseUrl: '',
    wireApi: 'openai-chat',
    keyHint: '',
    modelSource: 'catalog',
    catalogFetched: null,
    stale: false,
    state: 'ready',
    enabled: true,
    defaultModelId: null,
    models: [],
    accounts: [],
    login: null,
    ...partial,
  }
}

export function mockProviders(): MockProvider[] {
  return [
    provider({
      id: 'claude-code', name: 'Claude Code', kind: 'subscription', family: 'claude-code',
      accounts: [
        { id: 'a1', label: 'zan@example.com', plan: 'Max 5×', active: true, quota: { used: 62, max: 100, resets: 'in 2 h 10 min' } },
        { id: 'a2', label: 'zan@work.example', plan: 'Pro', active: false, quota: { used: 100, max: 100, resets: 'in 4 h' } },
      ],
      defaultModelId: 'claude-sonnet-4-5',
      catalogFetched: '2 min ago',
      models: [
        model({ id: 'claude-opus-4-1', name: 'Claude Opus 4.1', contextWindow: 200_000, outputLimit: 32_000, tools: true, attachments: true, efforts: ['low', 'medium', 'high', 'max'], defaultEffort: 'high' }),
        model({ id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', contextWindow: 1_000_000, outputLimit: 64_000, tools: true, attachments: true, efforts: ['low', 'medium', 'high', 'max'], defaultEffort: 'medium', fastTier: 'fast' }),
        model({ id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', contextWindow: 200_000, outputLimit: 64_000, tools: true, attachments: true, efforts: [], enabled: false }),
      ],
    }),
    provider({
      id: 'codex', name: 'Codex', kind: 'subscription', family: 'codex', state: 'signed-out',
      login: { url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expires: '9 min' },
      catalogFetched: null,
      models: [
        model({ id: 'gpt-5-codex', name: 'GPT-5 Codex', contextWindow: 400_000, outputLimit: 128_000, tools: true, attachments: true, efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' }),
      ],
    }),
    provider({
      id: 'anthropic', name: 'Anthropic', kind: 'api_key', family: 'anthropic', vendorId: 'anthropic',
      baseUrl: 'https://api.anthropic.com', wireApi: 'anthropic-messages', keyHint: 'sk-ant-…3f2a',
      catalogFetched: '14 min ago', defaultModelId: 'claude-sonnet-4-5',
      models: [
        model({ id: 'claude-opus-4-1', name: 'Claude Opus 4.1', contextWindow: 200_000, outputLimit: 32_000, tools: true, attachments: true, efforts: ['low', 'medium', 'high'], defaultEffort: 'high' }),
        model({ id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', contextWindow: 1_000_000, outputLimit: 64_000, tools: true, attachments: true, efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' }),
        model({ id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', contextWindow: 200_000, outputLimit: 64_000, tools: true, attachments: true, efforts: [] }),
        model({ id: 'claude-3-5-sonnet-20241022', name: 'Claude 3.5 Sonnet', contextWindow: 200_000, outputLimit: 8_192, tools: true, attachments: true, efforts: [], enabled: false }),
      ],
    }),
    provider({
      id: 'openai', name: 'OpenAI', kind: 'api_key', family: 'openai', vendorId: 'openai',
      baseUrl: 'https://api.openai.com/v1', wireApi: 'openai-responses', keyHint: 'sk-proj-…91ce',
      state: 'error', detail: '401 · Incorrect API key provided', catalogFetched: '3 days ago', stale: true,
      models: [
        model({ id: 'gpt-5', name: 'GPT-5', contextWindow: 400_000, outputLimit: 128_000, tools: true, attachments: true, efforts: ['minimal', 'low', 'medium', 'high'], defaultEffort: 'medium', fastTier: 'priority' }),
        model({ id: 'gpt-5-mini', name: 'GPT-5 mini', contextWindow: 400_000, outputLimit: 128_000, tools: true, attachments: true, efforts: ['minimal', 'low', 'medium', 'high'], defaultEffort: 'low' }),
      ],
    }),
    provider({
      id: 'kimi', name: 'Kimi', kind: 'api_key', family: 'anthropic', vendorId: null,
      baseUrl: 'https://api.moonshot.cn/anthropic', wireApi: 'anthropic-messages', keyHint: 'sk-…8c1d',
      modelSource: 'manual', defaultModelId: 'kimi-k2-thinking',
      models: [
        model({ id: 'kimi-k2-thinking', name: 'Kimi K2 Thinking', contextWindow: 256_000, outputLimit: 32_000, tools: true, attachments: false, efforts: ['low', 'high'], defaultEffort: 'high' }),
        model({ id: 'kimi-k2-turbo-preview' }),
      ],
    }),
    provider({
      id: 'ollama', name: 'Ollama', kind: 'api_key', family: 'openai', vendorId: null,
      baseUrl: 'http://localhost:11434/v1', wireApi: 'openai-chat', keyHint: '',
      modelSource: 'manual', state: 'unreachable', detail: 'connect ECONNREFUSED 127.0.0.1:11434',
      models: [model({ id: 'qwen3:32b', contextWindow: 40_000, tools: true, attachments: false })],
    }),
    provider({
      id: 'vertex', name: 'Google Vertex', kind: 'api_key', family: 'google', vendorId: 'google-vertex',
      baseUrl: 'https://us-central1-aiplatform.googleapis.com', wireApi: 'openai-chat', keyHint: 'ya29.…', state: 'disabled', enabled: false,
      catalogFetched: '1 h ago',
      models: [model({ id: 'gemini-2.5-pro', name: 'Gemini 2.5 Pro', contextWindow: 1_000_000, outputLimit: 65_536, tools: true, attachments: true, efforts: ['low', 'high'] })],
    }),
  ]
}

export function createSettingsState() {
  return reactive({
    general: {
      language: 'English',
      theme: 'system' as 'light' | 'dark' | 'system',
      accent: 'Blue',
      density: 'regular' as 'compact' | 'regular' | 'comfortable',
      fontSize: 15,
      sendWith: 'enter' as 'enter' | 'cmdEnter',
      openAtLogin: true,
      channel: 'Stable',
      version: '1.6.2',
    },
    notifications: {
      desktop: true,
      sound: false,
      onFinish: true,
      onApproval: true,
      onError: true,
      quietHours: true,
      quietRange: '22:00 – 08:00',
    },
    account: {
      name: 'Zan',
      email: 'zan@example.com',
      plan: 'Pro',
      renews: 'Oct 12',
      creditsUsed: 8_640,
      creditsMax: 10_000,
    },
    providers: mockProviders(),
    selectedProviderId: 'kimi' as string | null,
    providerDetailOpen: false,
    defaults: {
      chat: 'Claude Sonnet 4.5',
      fast: 'Claude Haiku 4.5',
      subagent: 'GPT-5 mini',
      reasoning: 'medium' as 'off' | 'low' | 'medium' | 'high',
      temperature: 0.3,
    },
    agents: [
      { id: 'build', name: 'Build', model: 'Claude Sonnet', mode: 'primary', summary: 'Full tool access · edits, shell, web', isDefault: true, enabled: true, expanded: true },
      { id: 'plan', name: 'Plan', model: 'Claude Sonnet', mode: 'primary', summary: 'Read-only · asks before every edit', enabled: true, expanded: false },
      { id: 'explore', name: 'Explore', model: 'Claude Haiku', mode: 'subagent', summary: 'Search and read · no writes', enabled: true, expanded: false },
      { id: 'review', name: 'Review', model: 'GPT-5', mode: 'subagent', summary: 'Custom prompt · 2,140 tokens', enabled: false, expanded: false },
    ] as MockAgent[],
    permissions: {
      edit: 'ask' as Permission,
      shell: 'ask' as Permission,
      read: 'allow' as Permission,
      web: 'allow' as Permission,
      mcp: 'ask' as Permission,
      scope: 'project' as 'project' | 'everywhere',
      allowlist: ['git *', 'bun test *', 'npm run *', 'ls *'],
    },
    instructions: {
      global: 'Prefer small, reviewable changes. Run the relevant tests before reporting done. Never call real models from tests.',
      files: [
        { path: '~/Projects/demi/AGENTS.md', found: true },
        { path: '~/Projects/demi/packages/web/AGENTS.md', found: true },
        { path: '~/Projects/notes/AGENTS.md', found: false },
      ],
      memory: true,
      memories: 12,
    },
    servers: [
      {
        id: 'github', name: 'github', transport: 'stdio', target: 'npx @modelcontextprotocol/server-github', state: 'connected', enabled: true, expanded: true,
        tools: [
          { name: 'create_issue', enabled: true },
          { name: 'list_pull_requests', enabled: true },
          { name: 'merge_pull_request', enabled: false },
          { name: 'search_code', enabled: true },
        ],
      },
      { id: 'postgres', name: 'postgres', transport: 'http', target: 'https://mcp.internal/pg', state: 'auth', enabled: true, expanded: false, detail: 'Sign in to authorize this server', tools: [] },
      { id: 'filesystem', name: 'filesystem', transport: 'stdio', target: 'npx @modelcontextprotocol/server-filesystem', state: 'crashed', enabled: true, expanded: false, detail: 'exit code 1 · npx: command not found', tools: [] },
      { id: 'sentry', name: 'sentry', transport: 'http', target: 'https://mcp.sentry.dev', state: 'disabled', enabled: false, expanded: false, tools: [] },
    ] as MockServer[],
    devices: [
      { id: 'mac', name: 'zan-mbp', online: true, current: true, home: '/Users/zan', version: '1.6.2', seen: 'Now' },
      { id: 'build', name: 'build-01', online: false, current: false, home: '/home/build', version: '1.5.9', seen: '3 days ago' },
      { id: 'lab', name: 'lab-workstation-with-a-long-hostname', online: true, current: false, home: '/home/lab', version: '1.6.2', seen: '2 minutes ago' },
    ],
    keys: [
      { id: 'new', action: 'New conversation', keys: '⌘N' },
      { id: 'send', action: 'Send message', keys: '⏎' },
      { id: 'stop', action: 'Stop the turn', keys: '⎋' },
      { id: 'sidebar', action: 'Toggle sidebar', keys: '⌘B' },
      { id: 'search', action: 'Search conversations', keys: '⌘K' },
      { id: 'focus', action: 'Focus composer', keys: '⌘K', conflict: 'Search conversations' },
      { id: 'settings', action: 'Open settings', keys: '⌘,' },
    ],
    data: {
      retention: 'Forever',
      shareLinks: false,
      telemetry: true,
    },
    usage: {
      spend: [
        { provider: 'Anthropic', amount: '$12.40' },
        { provider: 'OpenAI', amount: '$3.10' },
        { provider: 'Ollama', amount: '—' },
      ],
      input: '1.2M',
      output: '210k',
      cacheRead: '890k',
      resets: 'Oct 1',
    },
    developer: {
      logLevel: 'Info',
      configPath: '~/.config/demi/config.json',
      experiments: [
        { id: 'parallel', name: 'Parallel tool calls', description: 'Run independent tool calls at once.', on: true },
        { id: 'background', name: 'Background agents', description: 'Keep subagents running after the turn ends.', on: false },
        { id: 'voice', name: 'Voice input', description: 'Dictate into the composer.', on: false },
      ],
      env: [
        { key: 'ANTHROPIC_API_KEY', value: 'sk-ant-••••••••••••3f2a' },
        { key: 'DEMI_LOG_DIR', value: '~/Library/Logs/Demi' },
      ],
    },
  })
}

export type SettingsState = ReturnType<typeof createSettingsState>
