import { reactive } from 'vue'
import { createQuotaRefreshCache } from '@demicodes/web-ui/settings/quota-refresh'
import type { ExposeMenuEntry } from '@demicodes/plugin-expose/types'
import type {
  SettingsArchivedConversation,
  SettingsDevice,
  SettingsKeyBinding,
  SettingsPlugin,
  SettingsProviderEntry,
  SettingsProviderModel,
  SettingsVendor
} from '@demicodes/web-ui/settings/types'
import type { ExposeState } from '@demicodes/plugin-expose'
import type { SkillsState, SourceState } from '@demicodes/plugin-skills'
import { commitOf } from './plugins'
import { createSubagentState } from './subagent-profiles'
import { ago, ahead } from './time'
import { demoDeviceStart } from './device-installation'

/**
 * A coding agent's whole settings surface, mocked in every awkward state at once:
 * expiring auth, a crashed server, a full quota, a disabled provider.
 */

/** The exposes the session tools specimens list: two on `zan-mbp` (one under a minute), one on Cloud. */
export function demoExposes(): ExposeMenuEntry[] {
  return [
    {
      id: 'k7x2maqw4p3s6tavaw2y4z6aab',
      address: '127.0.0.1:5173',
      hostName: 'zan-mbp',
      url: 'https://k7x2maqw4p3s6tavaw2y4z6aab.expose.demi.example/',
      expiresAt: ahead(52 * 60_000),
    },
    {
      id: 'q7w6e7r6t5y4u3i2o3p2a4s2d3',
      address: '127.0.0.1:3000',
      hostName: 'zan-mbp',
      url: 'https://q7w6e7r6t5y4u3i2o3p2a4s2d3.expose.demi.example/',
      expiresAt: ahead(45_000),
    },
    {
      id: 'm3n5p7rgtxv3w5x7yez4a3c5ek',
      address: '127.0.0.1:8080',
      hostName: 'Cloud',
      url: 'https://m3n5p7rgtxv3w5x7yez4a3c5ek.expose.demi.example/',
      expiresAt: ahead(9 * 60_000),
    },
  ]
}
/**
 * `demoExposes` as the expose plugin's state gives them: on the gallery's
 * `mac` host, named `zan-mbp`, and on its Cloud.
 */
export function demoExposeState(): ExposeState {
  const devices: Record<string, string> = { 'zan-mbp': 'mac', Cloud: 'managed-device' }
  return {
    available: true,
    exposes: demoExposes().map((expose, index) => ({
      id: expose.id,
      number: index + 1,
      deviceId: devices[expose.hostName]!,
      deviceName: expose.hostName,
      address: expose.address,
      url: expose.url,
      expiresAt: expose.expiresAt,
    })),
  }
}

/** A skill of the Skills showcase: on unless `enabled` says otherwise, offered to the agent, no warning. */
function showcaseSkill(name: string, description: string, change: Partial<SourceState['skills'][number]> = {}): SourceState['skills'][number] {
  return { name, description, warnings: [], enabled: true, disableModelInvocation: false, ...change }
}

/** A source of the Skills showcase, idle at a fetched commit unless `change` says otherwise. */
function showcaseSource(id: string, change: Partial<SourceState> & Pick<SourceState, 'skills'>): SourceState {
  return {
    id,
    origin: `acme/${id}`,
    commit: commitOf(id),
    fetchedAt: ahead(-3 * 60 * 60_000),
    fetching: false,
    updateAvailable: false,
    skipped: [],
    ...change,
  }
}

/**
 * Two packs whose `review` skills share a name: the first's is on, so
 * turning the second's on is refused, as the backend's plugin refuses it.
 */
export function skillsCalls(): SkillsState {
  return {
    sources: [
      showcaseSource('review-kit', {
        skills: [
          showcaseSkill('review', 'Review a change before it lands.'),
          showcaseSkill('commit-message', 'A conventional commit from the staged diff.'),
        ],
      }),
      showcaseSource('web-kit', {
        skills: [
          showcaseSkill('design-review', 'Review UI against the web interface guidelines.'),
          showcaseSkill('review', 'Review a component and its specimens.', { enabled: false }),
        ],
      }),
    ],
  }
}

/** The packs of `skillsCalls` shown open. */
export const SKILLS_CALLS_OPEN = ['web-kit'] as const

/**
 * Every state of a skill source and of a skill, pinned at once: sources all
 * on, some on, all off, updating, with an update, failed after a good fetch,
 * failed at the first fetch, and with skipped files; skills on, off, with a
 * warning, of a taken name, and never offered to the agent.
 */
export function skillsShowcase(): SkillsState {
  return {
    sources: [
      showcaseSource('review-kit', {
        skills: [
          showcaseSkill('review', 'Review a change before it lands.'),
          showcaseSkill('commit-message', 'A conventional commit from the staged diff.'),
        ],
      }),
      showcaseSource('web-kit', {
        skills: [
          showcaseSkill('design-review', 'Review UI against the web interface guidelines.'),
          showcaseSkill('perf-audit', 'Find the slow paths of a page load.', { enabled: false }),
          showcaseSkill('copy-edit', 'Tighten interface copy.', { warnings: ['the name "Copy_Edit" breaks the name rule'] }),
          showcaseSkill('review', 'Review a component and its specimens.', { enabled: false }),
          showcaseSkill('release-notes', 'Write the release notes the user asks for.', { enabled: false, disableModelInvocation: true }),
        ],
      }),
      showcaseSource('archive', {
        skills: [
          showcaseSkill('legacy-deploy', 'Deploy with the old pipeline.', { enabled: false }),
          showcaseSkill('legacy-lint', 'Lint with the old rules.', { enabled: false }),
        ],
      }),
      showcaseSource('agent-skills', {
        fetching: true,
        skills: [
          showcaseSkill('tdd', 'Write the failing test before the change.'),
          showcaseSkill('debug', 'Narrow a failure down to its cause.', { enabled: false }),
        ],
      }),
      showcaseSource('platform-infrastructure-skills', {
        updateAvailable: true,
        skills: [showcaseSkill('terraform-plan', 'Read a plan before it applies.')],
      }),
      showcaseSource('flaky-skills', {
        failure: { at: ahead(-20 * 60_000), message: 'Could not connect to github.com: the connection timed out' },
        skills: [showcaseSkill('triage', 'Sort new issues by area and urgency.')],
      }),
      showcaseSource('missing', {
        commit: undefined,
        fetchedAt: undefined,
        failure: { at: ahead(-5 * 60_000), message: 'Repository not found' },
        skills: [],
      }),
      showcaseSource('drafts', {
        skills: [showcaseSkill('outline', 'Outline a document before writing it.')],
        skipped: [
          { path: 'skills/draft/SKILL.md', reason: 'the front matter has no description' },
          { path: 'skills/huge/SKILL.md', reason: 'the file is larger than 256 KiB' },
        ],
      }),
    ],
  }
}

/** The showcase's sources shown open: the one with every skill state, and the one with skipped files. */
export const SKILLS_SHOWCASE_OPEN = ['web-kit', 'drafts'] as const

/** The shared model plus what the mock knows but the page does not show. */
export interface MockModel extends SettingsProviderModel {
  tools: boolean | null
}

export interface MockProvider extends SettingsProviderEntry {
  /** Runtime family; subscriptions name their CLI vendor. */
  family: string
  models: MockModel[]
}

function model(partial: Partial<MockModel> & Pick<MockModel, 'id'>): MockModel {
  return {
    name: '',
    contextWindow: null,
    outputLimit: null,
    tools: null,
    extensions: [],
    efforts: [],
    fastTier: null,
    enabled: true,
    ...partial,
  }
}

export function provider(
  partial: Partial<MockProvider> & Pick<MockProvider, 'id' | 'name' | 'kind' | 'family'>
): MockProvider {
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
    autoRefreshUsage: partial.kind === 'subscription',
    logo: null,
    ...partial,
  }
}

/** models.dev vendors one of Demi's runtimes can speak to, with their marks. */
export interface MockVendor extends SettingsVendor {
  family: 'anthropic' | 'openai' | 'google'
}

export const mockVendors: MockVendor[] = [
  {
    id: 'anthropic',
    name: 'Anthropic',
    family: 'anthropic',
    wireApi: 'anthropic-messages',
    baseUrl: 'https://api.anthropic.com/v1',
    logo: '/logos/anthropic.svg'
  },
  {
    id: 'openai',
    name: 'OpenAI',
    family: 'openai',
    wireApi: 'openai-responses',
    baseUrl: 'https://api.openai.com/v1',
    logo: '/logos/openai.svg'
  },
  {
    id: 'google',
    name: 'Google',
    family: 'google',
    wireApi: 'openai-chat',
    baseUrl: null,
    logo: '/logos/google.svg'
  },
  {
    id: 'google-vertex',
    name: 'Vertex',
    family: 'google',
    wireApi: 'openai-chat',
    baseUrl: null,
    logo: '/logos/google-vertex.svg'
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://api.deepseek.com',
    logo: '/logos/deepseek.svg'
  },
  {
    id: 'moonshotai',
    name: 'Moonshot AI',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://api.moonshot.ai/v1',
    logo: '/logos/moonshotai.svg'
  },
  {
    id: 'zhipuai',
    name: 'Zhipu AI',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    logo: '/logos/zhipuai.svg'
  },
  {
    id: 'minimax',
    name: 'MiniMax',
    family: 'anthropic',
    wireApi: 'anthropic-messages',
    baseUrl: 'https://api.minimax.io/anthropic/v1',
    logo: '/logos/minimax.svg'
  },
  {
    id: 'fireworks-ai',
    name: 'Fireworks AI',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://api.fireworks.ai/inference/v1',
    logo: '/logos/fireworks-ai.svg'
  },
  {
    id: 'alibaba',
    name: 'Alibaba',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1',
    logo: '/logos/alibaba.svg'
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://openrouter.ai/api/v1',
    logo: '/logos/openrouter.svg'
  },
]

export function mockProviders(): MockProvider[] {
  return [
    provider({
      id: 'claude-code', name: 'Claude Code', kind: 'subscription', family: 'claude-code', logo: '/logos/claude.svg',
      // Two machines answered; the Cloud install after the last account failed, which is the one
      // state here that asks the user for anything.
      cli: {
        newest: { version: '2.1.278' },
        install: { state: 'failed', message: 'Claude Code 2.1.278 could not be installed: the download stopped after 120 MB' },
        machines: [
          { id: 'cloud', name: 'Cloud', versions: ['2.1.267'] },
          { id: 'laptop', name: 'MacBook Pro', versions: [] },
        ],
      },
      accounts: [
        {
          id: 'a1',
          label: 'zan@example.com',
          plan: 'Max 5×',
          active: true,
          quota: [
 { id: 'hour', label: '5-hour',
              used: 62,
              max: 100,
              resets: 'in 2 h 10 min'
             },
 { id: 'week', label: 'Weekly',  used: 31, max: 100, resets: 'Monday'  },
]
        },
        {
          id: 'a2',
          label: 'zan@work.example',
          plan: 'Pro',
          active: false,
          quota: [
 { id: 'hour', label: '5-hour',
              used: 100,
              max: 100,
              resets: 'in 4 h'
             },
 { id: 'week', label: 'Weekly',
              used: 88,
              max: 100,
              resets: 'Thursday'
             },
]
        },
        // Signed in with a token: the vendor names no plan, so the row shows none.
        {
          id: 'a3',
          label: 'claude-c3bd1c21',
          plan: '',
          active: false,
          // A vendor names its windows as it likes; the label column fits the longest.
          quota: [
            { id: 'requests', label: 'Requests (short window)', used: 12, max: 100, resets: 'in 40 s' },
            { id: 'tokens', label: 'Tokens (short window)', used: 3, max: 100, resets: 'in 40 s' },
          ],
        },
        // Long text everywhere: the tags move under the name, which is cut only where the line
        // cannot hold it, and the window names and reset times wrap inside the text column
        // instead of running under the buttons.
        {
          id: 'a4',
          label: 'release-automation@platform-infrastructure.example.com',
          plan: 'Max 20×',
          active: false,
          quota: [
            {
              id: 'week-all',
              label: 'Weekly (all models, including Opus)',
              used: 100,
              max: 100,
              resets: 'on Thursday, October 9 at 2:00 PM',
            },
            { id: 'week-sonnet', label: 'Weekly (Sonnet only)', used: 47, max: 100, resets: 'in 6 days 23 h' },
          ],
        },
      ],
      catalogFetched: '2 min ago',
      models: [
        model(
          {
            id: 'claude-opus-4-1',
            name: 'Claude Opus 4.1',
            contextWindow: 200_000,
            outputLimit: 32_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: ['low', 'medium', 'high', 'max']
          }
        ),
        model(
          {
            id: 'claude-sonnet-4-5',
            name: 'Claude Sonnet 4.5',
            contextWindow: 1_000_000,
            outputLimit: 64_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: ['low', 'medium', 'high', 'max'],
            fastTier: 'fast'
          }
        ),
        model(
          {
            id: 'claude-haiku-4-5',
            name: 'Claude Haiku 4.5',
            contextWindow: 200_000,
            outputLimit: 64_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: [],
            enabled: false
          }
        ),
      ],
    }),
    provider({
      id: 'codex', name: 'Codex', kind: 'subscription', family: 'codex', state: 'signed-out', logo: '/logos/openai.svg',
      catalogFetched: null,
      models: [
        model(
          {
            id: 'gpt-5-codex',
            name: 'GPT-5 Codex',
            contextWindow: 400_000,
            outputLimit: 128_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: ['low', 'medium', 'high']
          }
        ),
      ],
    }),
    provider(
      {
        id: 'grok-build',
        name: 'Grok Build',
        kind: 'subscription',
        family: 'grok-build',
        logo: '/logos/xai.svg',
        // Signed in and reachable from the backend: its active account can be tested.
        accounts: [
          { id: 'g1', label: 'zan@example.com', plan: 'XPremium', active: true, quota: [] },
          { id: 'g2', label: 'zan@work.example', plan: '', active: false, quota: [] },
        ],
        models: [
          model({ id: 'grok-4-6', name: 'Grok 4.6', contextWindow: 256_000, outputLimit: 64_000, tools: true }),
        ],
      }
    ),
    provider({
      id: 'anthropic', name: 'Anthropic', kind: 'api_key', family: 'anthropic', vendorId: 'anthropic', logo: '/logos/anthropic.svg',
      baseUrl: 'https://api.anthropic.com', wireApi: 'anthropic-messages', apiKey: '', keyConfigured: true, testedIn: '412 ms',
      catalogFetched: '14 min ago',
      models: [
        model(
          {
            id: 'claude-opus-4-1',
            name: 'Claude Opus 4.1',
            contextWindow: 200_000,
            outputLimit: 32_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: ['low', 'medium', 'high']
          }
        ),
        model(
          {
            id: 'claude-sonnet-4-5',
            name: 'Claude Sonnet 4.5',
            contextWindow: 1_000_000,
            outputLimit: 64_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: ['low', 'medium', 'high']
          }
        ),
        model(
          {
            id: 'claude-haiku-4-5',
            name: 'Claude Haiku 4.5',
            contextWindow: 200_000,
            outputLimit: 64_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: []
          }
        ),
        model(
          {
            id: 'claude-3-5-sonnet-20241022',
            name: 'Claude 3.5 Sonnet',
            contextWindow: 200_000,
            outputLimit: 8_192,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: [],
            enabled: false
          }
        ),
      ],
    }),
    provider({
      id: 'openai', name: 'OpenAI', kind: 'api_key', family: 'openai', vendorId: 'openai', logo: '/logos/openai.svg',
      baseUrl: 'https://api.openai.com/v1', wireApi: 'openai-responses', apiKey: 'sk-proj-91ce4a7b2d8f6e1c3a5b9d7f',
      state: 'error', detail: '401 · Incorrect API key provided', catalogFetched: '3 days ago', stale: true,
      models: [
        model(
          {
            id: 'gpt-5',
            name: 'GPT-5',
            contextWindow: 400_000,
            outputLimit: 128_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: ['minimal', 'low', 'medium', 'high'],
            fastTier: 'priority'
          }
        ),
        model(
          {
            id: 'gpt-5-mini',
            name: 'GPT-5 mini',
            contextWindow: 400_000,
            outputLimit: 128_000,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: ['minimal', 'low', 'medium', 'high']
          }
        ),
      ],
    }),
    provider({
      id: 'kimi', name: 'Kimi', kind: 'api_key', family: 'anthropic', vendorId: 'moonshotai', logo: '/logos/moonshotai.svg',
      baseUrl: 'https://api.moonshot.cn/anthropic', wireApi: 'anthropic-messages', apiKey: 'sk-8c1d5e2f9a7b3c6d4e1f0a9b', testedIn: '412 ms',
      modelSource: 'manual',
      models: [
        model(
          {
            id: 'kimi-k2-thinking',
            name: 'Kimi K2 Thinking',
            contextWindow: 256_000,
            outputLimit: 32_000,
            tools: true,
            extensions: [],
            efforts: ['low', 'high']
          }
        ),
        model({ id: 'kimi-k2-turbo-preview' }),
      ],
    }),
    provider({
      id: 'ollama', name: 'Ollama on the lab workstation', kind: 'api_key', family: 'openai', vendorId: null,
      baseUrl: 'http://localhost:11434/v1', wireApi: 'openai-chat', apiKey: '',
      modelSource: 'manual', state: 'unreachable', detail: 'connect ECONNREFUSED 127.0.0.1:11434',
      models: [
        model({
          id: 'qwen3:32b',
          contextWindow: 40_000,
          tools: true,
          extensions: []
        })
      ],
    }),
    provider({
      id: 'vertex', name: 'Google Vertex', kind: 'api_key', family: 'google', vendorId: 'google-vertex', logo: '/logos/google-vertex.svg',
      baseUrl: 'https://us-central1-aiplatform.googleapis.com', wireApi: 'openai-chat', apiKey: 'ya29.a0AfB_byC1d2E3f4G5h6', state: 'disabled', enabled: false,
      catalogFetched: '1 h ago',
      models: [
        model(
          {
            id: 'gemini-2.5-pro',
            name: 'Gemini 2.5 Pro',
            contextWindow: 1_000_000,
            outputLimit: 65_536,
            tools: true,
            extensions: [
              '.png',
              '.jpg',
              '.jpeg',
              '.gif',
              '.webp',
              '.pdf'
            ],
            efforts: ['low', 'high']
          }
        )
      ],
    }),
  ]
}

/** The paired devices the devices page lists. */
function galleryDevices(): SettingsDevice[] {
  return [
    { id: 'mac', name: 'zan-mbp', state: 'online', seen: ago(0), direct: 'connected' },
    { id: 'build', name: 'build-01', state: 'offline', seen: ago(3 * 24 * 60 * 60 * 1000), start: demoDeviceStart('linux') },
    { id: 'studio', name: 'studio-pc', state: 'offline', seen: ago(24 * 60 * 60 * 1000), start: demoDeviceStart('windows') },
    {
      id: 'lab',
      name: 'lab-workstation-with-a-long-hostname',
      state: 'updating',
      seen: ago(2 * 60 * 1000),
    },
  ]
}

/** The projects on the paired devices, which a revoked device takes with it. */
function galleryDeviceProjects(): { deviceId: string; name: string }[] {
  return [
    { deviceId: 'mac', name: 'demi' },
    { deviceId: 'mac', name: 'notes' },
  ]
}

export function createSettingsState() {
  return reactive({
    quotaRefreshCache: createQuotaRefreshCache(),
    general: {
      language: 'English',
      theme: 'system' as 'light' | 'dark' | 'system',
      fontSize: 15,
    },
    notifications: {
      webBrowser: true,
      sound: false,
      onFinish: true,
      onError: true,
    },
    account: {
      name: 'Zan',
      email: 'zan@example.com',
      passwordChanged: '3 months ago',
    },
    providers: mockProviders(),
    selectedProviderId: null as string | null,
    providerDetailOpen: false,
    plugins: [
      { id: 'file', name: 'Demi File Commands', description: 'Reads, writes, edits and searches the conversation’s files with demi file.', enabled: true },
      {
        id: 'browser',
        name: 'Browser',
        description: 'A browser on the conversation’s host that the agent drives with demi browser and the user watches in the work panel.',
        enabled: true,
      },
      { id: 'expose', name: 'Expose', description: 'Gives a service on one of your hosts a public URL for an hour, with demi expose.', enabled: false },
      {
        id: 'skills',
        name: 'Skills',
        description: 'Workflows the agent follows: skills from Git repositories you add, and those your repository carries.',
        enabled: true,
      },
      {
        id: 'changes',
        name: 'Changes',
        description: 'Shows the uncommitted changes in the conversation’s working directory, and what each of the agent’s commands changed, as diffs in the work panel.',
        enabled: true,
      },
      { id: 'file-browser', name: 'File Browser', description: 'Opens the conversation’s files in the work panel to read them, with a tree of the working directory.', enabled: true },
    ] as SettingsPlugin[],
    subagents: createSubagentState(),
    skills: {
      sources: [
        {
          id: 'vercel',
          origin: 'https://github.com/vercel-labs/agent-skills',
          commit: '9f2c1a7b9f2c1a7b9f2c1a7b9f2c1a7b9f2c1a7b',
          fetchedAt: ahead(-2 * 24 * 60 * 60_000),
          fetching: false,
          updateAvailable: false,
          skills: [
            { name: 'web-design-guidelines', description: 'Review UI against Vercel’s web interface guidelines.', warnings: [], enabled: true, disableModelInvocation: false },
            { name: 'vercel-react-best-practices', description: 'React composition and data-fetching patterns.', warnings: [], enabled: true, disableModelInvocation: false },
            { name: 'vercel-react-native-skills', description: 'React Native layout and navigation conventions.', warnings: [], enabled: false, disableModelInvocation: false },
            { name: 'vercel-composition-patterns', description: 'When to split a component and when to leave it.', warnings: ['the description is longer than 1,024 characters'], enabled: true, disableModelInvocation: false },
            { name: 'frontend-design', description: 'Taste-led interface work: type, color, motion.', warnings: [], enabled: false, disableModelInvocation: false },
            { name: 'tdd', description: 'Write the failing test before the change.', warnings: [], enabled: false, disableModelInvocation: false },
            { name: 'agent-browser', description: 'Browse and act on a page the agent can see.', warnings: [], enabled: false, disableModelInvocation: false },
            { name: 'find-skills', description: 'Search installed skills when the next step is unclear.', warnings: [], enabled: true, disableModelInvocation: true },
          ],
          skipped: [{ path: 'skills/draft/SKILL.md', reason: 'the front matter has no description' }],
        },
        {
          id: 'anthropic',
          origin: 'https://github.com/anthropics/skills',
          commit: '9f2c1a7c9f2c1a7c9f2c1a7c9f2c1a7c9f2c1a7c',
          fetchedAt: ahead(-2 * 24 * 60 * 60_000),
          fetching: false,
          updateAvailable: true,
          skills: [
            { name: 'pptx', description: 'Create and edit PowerPoint decks.', warnings: [], enabled: true, disableModelInvocation: false },
            { name: 'pdf', description: 'Read and fill PDF forms.', warnings: [], enabled: true, disableModelInvocation: false },
            { name: 'xlsx', description: 'Build spreadsheets from tables.', warnings: [], enabled: true, disableModelInvocation: false },
            { name: 'docx', description: 'Draft Word documents.', warnings: [], enabled: false, disableModelInvocation: false },
            { name: 'skill-creator', description: 'Author a new SKILL.md that other agents can load.', warnings: [], enabled: true, disableModelInvocation: false },
          ],
          skipped: [],
        },
        {
          id: 'commit',
          origin: 'https://github.com/zan/commit',
          commit: '9f2c1a7d9f2c1a7d9f2c1a7d9f2c1a7d9f2c1a7d',
          fetchedAt: ahead(-2 * 24 * 60 * 60_000),
          fetching: false,
          updateAvailable: false,
          skills: [
            { name: 'commit', description: 'Conventional commit from the staged diff.', warnings: [], enabled: true, disableModelInvocation: false },
          ],
          skipped: [],
        },
        {
          id: 'broken',
          origin: 'https://github.com/example/broken-skills',
          fetching: false,
          updateAvailable: false,
          failure: { at: ahead(-60 * 60_000), message: 'Repository not found' },
          skills: [
          ],
          skipped: [],
        },
      ],
    } as SkillsState,
    archived: [
      {
        id: 'oauth',
        title: 'OAuth refresh token rotation',
        detail: 'Archived Aug 2'
      },
      { id: 'icons', title: 'Icon theme survey', detail: 'Archived Jul 21' },
      {
        id: 'perf',
        title: 'Sidebar scroll performance',
        detail: 'Archived Jul 3'
      },
    ] satisfies SettingsArchivedConversation[],
    devices: galleryDevices(),
    deviceProjects: galleryDeviceProjects(),
    keys: [
      { id: 'new', action: 'New conversation', keys: '⌘⇧O' },
      { id: 'send', action: 'Send message', keys: '⏎' },
      { id: 'stop', action: 'Stop the turn', keys: '⎋' },
      { id: 'sidebar', action: 'Toggle sidebar', keys: '⌘B' },
      { id: 'search', action: 'Search conversations', keys: '⌘K' },
      { id: 'focus', action: 'Focus composer', keys: '⌘J' },
      { id: 'settings', action: 'Open settings', keys: '⌘,' },
    ] satisfies SettingsKeyBinding[],
    data: {
      shareLinks: false,
      telemetry: true,
    },
  })
}

export type SettingsState = ReturnType<typeof createSettingsState>
