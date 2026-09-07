import type { SettingsExperiment, SettingsKeyBinding, SettingsMcpServer, SettingsProviderEntry, SettingsProviderModel, SettingsSkillSource, SettingsVendor } from '@demicodes/web-ui/settings/types'
import type { ThemeChoice } from '@demicodes/web-ui/theme/appTheme'
import type { ProductAccent, ProductTone } from '@demicodes/web-ui/theme/productAppearance'

/**
 * The prototype's settings: what the pages edit, seeded the way a used account looks.
 * Providers double as the composer's model catalog, so a provider disabled here
 * disappears from the model menu.
 */

/** A provider with the runtime family its sign-in follows; the settings page never shows it. */
export interface PrototypeProvider extends SettingsProviderEntry {
  family: 'claude-code' | 'codex' | 'grok-build' | 'anthropic' | 'openai' | 'google' | 'custom'
}

export const LANGUAGES = ['English', '简体中文', '日本語']
export const RETENTIONS = ['Forever', '90 days', '30 days', '7 days']
export const LOG_LEVELS = ['Error', 'Warn', 'Info', 'Debug']

/** What each shortcut does in the app; ids are what `App` listens for. */
export const DEFAULT_KEYS: SettingsKeyBinding[] = [
  { id: 'new', action: 'New conversation', keys: '⌘N' },
  { id: 'sidebar', action: 'Toggle sidebar', keys: '⌘B' },
  { id: 'settings', action: 'Open settings', keys: '⌘,' },
]

export interface PrototypeSettings {
  general: { language: string; theme: ThemeChoice; tone: ProductTone; accent: ProductAccent; fontSize: number }
  notifications: { browser: boolean; sound: boolean; onFinish: boolean; onError: boolean }
  account: { email: string; passwordChanged: string; plan: string; renews: string; creditsUsed: number; creditsMax: number }
  servers: SettingsMcpServer[]
  skillSources: SettingsSkillSource[]
  keys: SettingsKeyBinding[]
  data: { retention: string; shareLinks: boolean; telemetry: boolean }
  usage: { spend: { provider: string; amount: string }[]; input: string; output: string; cacheRead: string; resets: string }
  developer: { logLevel: string; experiments: SettingsExperiment[] }
}

export function settings(): PrototypeSettings {
  return {
    general: { language: 'English', theme: 'system', tone: 'ink', accent: 'blue', fontSize: 15 },
    notifications: { browser: true, sound: false, onFinish: true, onError: true },
    account: { email: 'zan@example.com', passwordChanged: '3 months ago', plan: 'Pro', renews: 'Oct 12', creditsUsed: 8_640, creditsMax: 10_000 },
    servers: [
      {
        id: 'github', name: 'GitHub', transport: 'stdio', target: 'npx @modelcontextprotocol/server-github', state: 'connected', enabled: true,
        tools: ['create_issue', 'list_issues', 'create_pull_request', 'list_pull_requests', 'merge_pull_request', 'search_code', 'get_file', 'create_comment'],
      },
      { id: 'postgres', name: 'Postgres', transport: 'http', target: 'https://mcp.internal/pg', state: 'auth', enabled: true, detail: 'Sign in to authorize this server', tools: [] },
      { id: 'filesystem', name: 'FileSystem', transport: 'stdio', target: 'npx @modelcontextprotocol/server-filesystem', state: 'crashed', enabled: true, detail: 'exit code 1 · npx: command not found', tools: ['read_file', 'write_file', 'list_directory', 'search_files'] },
    ],
    skillSources: [
      {
        id: 'anthropic', name: 'anthropics/skills', origin: 'https://github.com/anthropics/skills', state: 'ready',
        skills: [
          { id: 'pptx', name: 'pptx', description: 'Create and edit PowerPoint decks.', enabled: true },
          { id: 'pdf', name: 'pdf', description: 'Read and fill PDF forms.', enabled: true },
          { id: 'xlsx', name: 'xlsx', description: 'Build spreadsheets from tables.', enabled: true },
          { id: 'docx', name: 'docx', description: 'Draft Word documents.', enabled: false },
        ],
      },
      {
        id: 'commit', name: 'zan/commit', origin: 'https://github.com/zan/commit', state: 'ready',
        skills: [{ id: 'commit', name: 'commit', description: 'Conventional commit from the staged diff.', enabled: true }],
      },
    ],
    keys: DEFAULT_KEYS.map((binding) => ({ ...binding })),
    data: { retention: 'Forever', shareLinks: false, telemetry: true },
    usage: {
      spend: [
        { provider: 'Anthropic', amount: '$12.40' },
        { provider: 'OpenAI', amount: '$3.10' },
        { provider: 'Ollama', amount: '—' },
      ],
      input: '1.2M', output: '210k', cacheRead: '890k', resets: 'Oct 1',
    },
    developer: {
      logLevel: 'Info',
      experiments: [
        { id: 'parallel', name: 'Parallel tool calls', description: 'Run independent tool calls at once.', on: true },
        { id: 'background', name: 'Background agents', description: 'Keep subagents running after the turn ends.', on: false },
      ],
    },
  }
}

/** models.dev vendors an API key can be added for. */
export const VENDORS: SettingsVendor[] = [
  { id: 'anthropic', name: 'Anthropic', wireApi: 'anthropic-messages', baseUrl: null, logo: '/logos/anthropic.svg' },
  { id: 'openai', name: 'OpenAI', wireApi: 'openai-responses', baseUrl: null, logo: '/logos/openai.svg' },
  { id: 'google', name: 'Google', wireApi: 'openai-chat', baseUrl: null, logo: '/logos/google.svg' },
  { id: 'deepseek', name: 'DeepSeek', wireApi: 'openai-chat', baseUrl: 'https://api.deepseek.com', logo: '/logos/deepseek.svg' },
  { id: 'moonshotai', name: 'Moonshot AI', wireApi: 'openai-chat', baseUrl: 'https://api.moonshot.ai/v1', logo: '/logos/moonshotai.svg' },
  { id: 'openrouter', name: 'OpenRouter', wireApi: 'openai-chat', baseUrl: 'https://openrouter.ai/api/v1', logo: '/logos/openrouter.svg' },
]

/** The family a vendor's key runs under. */
export function vendorFamily(vendorId: string): PrototypeProvider['family'] {
  if (vendorId === 'anthropic') return 'anthropic'
  if (vendorId === 'google') return 'google'
  return 'openai'
}

const CLAUDE_EXTENSIONS = ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf']

function model(partial: Partial<SettingsProviderModel> & Pick<SettingsProviderModel, 'id'>): SettingsProviderModel {
  return { name: '', contextWindow: null, outputLimit: null, extensions: [], efforts: [], fastTier: null, enabled: true, ...partial }
}

export function provider(partial: Partial<PrototypeProvider> & Pick<PrototypeProvider, 'id' | 'name' | 'kind' | 'family'>): PrototypeProvider {
  return {
    vendorId: null, baseUrl: '', wireApi: 'openai-chat', apiKey: '', modelSource: 'catalog', catalogFetched: null, stale: false,
    state: 'ready', enabled: true, models: [], accounts: [], logo: null,
    ...partial,
  }
}

export function providers(): PrototypeProvider[] {
  return [
    provider({
      id: 'claude-code', name: 'Claude Code', kind: 'subscription', family: 'claude-code', logo: '/logos/claude.svg', catalogFetched: '2 min ago',
      accounts: [{ id: 'a1', label: 'zan@example.com', plan: 'Max 5×', active: true, quota: { hour: { used: 62, max: 100, resets: 'in 2 h 10 min' }, week: { used: 31, max: 100, resets: 'Monday' } } }],
      models: [
        model({ id: 'claude-opus-4-1', name: 'Claude Opus 4.1', contextWindow: 200_000, outputLimit: 32_000, extensions: CLAUDE_EXTENSIONS, efforts: ['low', 'medium', 'high', 'max'] }),
        model({ id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', contextWindow: 1_000_000, outputLimit: 64_000, extensions: CLAUDE_EXTENSIONS, efforts: ['low', 'medium', 'high', 'max'], fastTier: 'fast' }),
        model({ id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', contextWindow: 200_000, outputLimit: 64_000, extensions: CLAUDE_EXTENSIONS, enabled: false }),
      ],
    }),
    provider({ id: 'codex', name: 'Codex', kind: 'subscription', family: 'codex', state: 'signed-out', logo: '/logos/openai.svg',
      models: [model({ id: 'gpt-5-codex', name: 'GPT-5 Codex', contextWindow: 400_000, outputLimit: 128_000, extensions: CLAUDE_EXTENSIONS, efforts: ['low', 'medium', 'high'] })],
    }),
    provider({ id: 'grok-build', name: 'Grok Build', kind: 'subscription', family: 'grok-build', state: 'signed-out', logo: '/logos/xai.svg' }),
    provider({
      id: 'anthropic', name: 'Anthropic', kind: 'api_key', family: 'anthropic', vendorId: 'anthropic', logo: '/logos/anthropic.svg',
      baseUrl: 'https://api.anthropic.com', wireApi: 'anthropic-messages', apiKey: 'sk-ant-api03-3f2a9c1d7e5b4a6f8c2d1e9b', testedIn: '412 ms', catalogFetched: '14 min ago',
      models: [
        model({ id: 'claude-opus-4-1', name: 'Claude Opus 4.1', contextWindow: 200_000, outputLimit: 32_000, extensions: CLAUDE_EXTENSIONS, efforts: ['low', 'medium', 'high'] }),
        model({ id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', contextWindow: 1_000_000, outputLimit: 64_000, extensions: CLAUDE_EXTENSIONS, efforts: ['low', 'medium', 'high'] }),
      ],
    }),
    provider({
      id: 'openai', name: 'OpenAI', kind: 'api_key', family: 'openai', vendorId: 'openai', logo: '/logos/openai.svg',
      baseUrl: 'https://api.openai.com/v1', wireApi: 'openai-responses', apiKey: 'sk-proj-91ce4a7b2d8f6e1c3a5b9d7f',
      state: 'error', detail: '401 · Incorrect API key provided', catalogFetched: '3 days ago', stale: true,
      models: [model({ id: 'gpt-5', name: 'GPT-5', contextWindow: 400_000, outputLimit: 128_000, extensions: CLAUDE_EXTENSIONS, efforts: ['minimal', 'low', 'medium', 'high'], fastTier: 'priority' })],
    }),
    provider({
      id: 'ollama', name: 'Ollama', kind: 'api_key', family: 'custom', vendorId: null, baseUrl: 'http://localhost:11434/v1', wireApi: 'openai-chat',
      modelSource: 'manual', state: 'unreachable', detail: 'connect ECONNREFUSED 127.0.0.1:11434',
      models: [model({ id: 'qwen3:32b', contextWindow: 40_000 })],
    }),
  ]
}

/** The provider and model a new conversation starts on. */
export const DEFAULT_MODEL = { providerId: 'claude-code', modelId: 'claude-sonnet-4-5' }
