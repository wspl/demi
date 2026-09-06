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
export type ProviderState = 'connected' | 'expiring' | 'unreachable' | 'incomplete' | 'disabled'
export type ServerState = 'connected' | 'auth' | 'crashed' | 'disabled'

export interface MockProvider {
  id: string
  name: string
  auth: string
  models: number
  state: ProviderState
  enabled: boolean
  detail?: string
  region?: string
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
    providers: [
      { id: 'anthropic', name: 'Anthropic', auth: 'API key', models: 6, state: 'connected', enabled: true },
      { id: 'openai', name: 'OpenAI', auth: 'OAuth', models: 4, state: 'expiring', enabled: true, detail: 'Token expires in 3 days' },
      { id: 'ollama', name: 'Ollama', auth: 'localhost:11434', models: 0, state: 'unreachable', enabled: true, detail: 'connect ECONNREFUSED 127.0.0.1:11434' },
      { id: 'bedrock', name: 'AWS Bedrock', auth: 'Instance profile', models: 0, state: 'incomplete', enabled: true, region: 'Choose a region' },
      { id: 'vertex', name: 'Google Vertex', auth: 'Service account', models: 3, state: 'disabled', enabled: false },
    ] as MockProvider[],
    defaults: {
      chat: 'Claude Sonnet',
      fast: 'Claude Haiku',
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
