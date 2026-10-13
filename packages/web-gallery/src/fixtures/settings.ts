import { DEMO_PERSONAL_INSTRUCTIONS } from './instructions'
import { reactive } from 'vue'
import { createQuotaRefreshCache } from '@demicodes/web-ui/settings/quota-refresh'
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import { providerLogo, vendorLogo } from '@demicodes/web-ui/settings/vendor-logos'
import { DEFAULT_TRANSCRIPT_TEXT_SIZE } from '@demicodes/web-ui/theme/productAppearance'
import type {
  SettingsArchivedConversation,
  SettingsKeyBinding,
  SettingsPlugin,
  SettingsProviderEntry,
  SettingsModelMatch,
  SettingsProviderModel,
  SettingsSetupPart,
  SettingsVendor
} from '@demicodes/web-ui/settings/types'
import type { SkillsState, SourceState } from '@demicodes/plugin-skills'
import type { WebState } from '@demicodes/plugin-web'
import { commitOf } from './plugins'
import { createCredentialState } from './credentials'
import { createUsersState } from './users'
import type { SettingsPasskey, SettingsSession } from '@demicodes/web-ui/settings/types'

const DAY_MS = 24 * 3_600_000

/** The account's passkeys: a synced one in iCloud Keychain, used today, and one Chrome kept on a Windows PC, never used. */
function galleryPasskeys(): SettingsPasskey[] {
  const ago = (ms: number) => new Date(Date.now() - ms).toISOString()
  return [
    { id: 'pk-icloud', name: 'iCloud Keychain', synced: true, createdAt: ago(41 * DAY_MS), lastUsedAt: ago(2 * 3_600_000) },
    { id: 'pk-windows', name: 'Chrome on Windows', synced: false, createdAt: ago(9 * DAY_MS), lastUsedAt: null },
  ]
}

/**
 * The browsers signed in to the account, newest first: a phone a code
 * signed in an hour ago, this Mac's Safari, and a Chrome on a Windows PC
 * last active three days ago.
 */
export function gallerySessions(): SettingsSession[] {
  const ago = (ms: number) => new Date(Date.now() - ms).toISOString()
  return [
    { id: 'session-phone', name: 'Safari on iPhone', createdAt: ago(3_600_000), lastActiveAt: ago(3_600_000), current: false, issuedBy: 'Ana' },
    { id: 'session-mac', name: 'Safari on macOS', createdAt: ago(12 * DAY_MS), lastActiveAt: ago(60_000), current: true, issuedBy: null },
    { id: 'session-windows', name: 'Chrome on Windows', createdAt: ago(20 * DAY_MS), lastActiveAt: ago(3 * DAY_MS), current: false, issuedBy: null },
  ]
}
import { createSubagentState } from './subagent-profiles'
import { DAY, HOUR, MINUTE, SECOND, ago, ahead } from './time'

/**
 * A coding agent's whole settings surface, mocked in every awkward state at once:
 * expiring auth, a crashed server, a full quota, a disabled provider.
 */

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
          showcaseSkill('copy-edit', 'Tighten interface copy.', { warnings: ['The name "Copy_Edit" breaks the name rule'] }),
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
    state: 'ready',
    enabled: true,
    models: [],
    accounts: [],
    autoRefreshUsage: partial.kind === 'subscription',
    logo: providerLogo({ kind: partial.kind, providerType: partial.family, vendorId: partial.vendorId ?? null }),
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
    logo: vendorLogo('anthropic')
  },
  {
    id: 'openai',
    name: 'OpenAI',
    family: 'openai',
    wireApi: 'openai-responses',
    baseUrl: 'https://api.openai.com/v1',
    logo: vendorLogo('openai')
  },
  {
    id: 'google',
    name: 'Google',
    family: 'google',
    wireApi: 'openai-chat',
    baseUrl: null,
    logo: vendorLogo('google')
  },
  {
    id: 'google-vertex',
    name: 'Vertex',
    family: 'google',
    wireApi: 'openai-chat',
    baseUrl: null,
    logo: vendorLogo('google-vertex')
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://api.deepseek.com',
    logo: vendorLogo('deepseek')
  },
  {
    id: 'moonshotai',
    name: 'Moonshot AI',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://api.moonshot.ai/v1',
    logo: vendorLogo('moonshotai')
  },
  {
    id: 'zhipuai',
    name: 'Zhipu AI',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    logo: vendorLogo('zhipuai')
  },
  {
    id: 'minimax',
    name: 'MiniMax',
    family: 'anthropic',
    wireApi: 'anthropic-messages',
    baseUrl: 'https://api.minimax.io/anthropic/v1',
    logo: vendorLogo('minimax')
  },
  {
    id: 'fireworks-ai',
    name: 'Fireworks AI',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://api.fireworks.ai/inference/v1',
    logo: vendorLogo('fireworks-ai')
  },
  {
    id: 'alibaba',
    name: 'Alibaba',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1',
    logo: vendorLogo('alibaba')
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    family: 'openai',
    wireApi: 'openai-chat',
    baseUrl: 'https://openrouter.ai/api/v1',
    logo: vendorLogo('openrouter')
  },
]

export function mockProviders(): MockProvider[] {
  return [
    provider({
      id: 'claude-code', name: 'Claude Code', kind: 'subscription', family: 'claude-code',
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
          label: 'alex@example.com',
          plan: 'Max 5×',
          active: true,
          // As the usage probe names them: by period and scope, a limit on one model too. The
          // accounts' resets show every wording a reset takes, as the clock counts them down:
          // in hours and minutes within a day, the weekday within 6 days, the day after that.
          quota: [
            { id: 'five_hour', label: '5h session', used: 62, max: 100, resetsAt: ahead(3 * HOUR + 42 * MINUTE + 30 * SECOND) },
            { id: 'seven_day', label: '7d all models', used: 31, max: 100, resetsAt: ahead(3 * DAY + 4 * HOUR) },
            { id: 'limit:weekly_scoped:Fable', label: '7d Fable', used: 74, max: 100, resetsAt: ahead(12 * DAY) },
          ],
        },
        {
          id: 'a2',
          label: 'alex@work.example',
          plan: 'Pro',
          active: false,
          // The full window's reset has passed and the next snapshot has not come yet.
          quota: [
            { id: 'five_hour', label: '5h session', used: 100, max: 100, resetsAt: ahead(-20 * SECOND) },
            { id: 'seven_day', label: '7d all models', used: 88, max: 100, resetsAt: ahead(5 * DAY) },
          ]
        },
        // The vendor names no plan, so the row shows none.
        {
          id: 'a3',
          label: 'alex@family.example',
          plan: '',
          active: false,
          // A vendor names its windows as it likes; the label column fits the longest. Its
          // usage comes as used ÷ limit × 100 in floating point, which reads in whole percent,
          // rounded down, so a window shows full only once it is. The short windows reset within
          // the minute and then read "now"; Daily resets on the hour, so its minutes are left
          // out, and a window whose reset the vendor does not report has no reset line.
          quota: [
            { id: 'requests', label: 'Requests (short window)', used: 7.000000000000001, max: 100, resetsAt: ahead(40 * SECOND) },
            { id: 'tokens', label: 'Tokens (short window)', used: 0.4, max: 100, resetsAt: ahead(42 * MINUTE + 30 * SECOND) },
            { id: 'daily', label: 'Daily', used: 98.6, max: 100, resetsAt: ahead(3 * HOUR + 30 * SECOND) },
            { id: 'monthly', label: 'Monthly credits', used: 23, max: 100, resetsAt: null },
          ],
        },
        // Long text everywhere: the tags move under the name, which is cut only where the line
        // cannot hold it, and the window names and reset times wrap inside the usage's own
        // line under the buttons.
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
              resetsAt: ahead(12 * DAY),
            },
            { id: 'week-sonnet', label: 'Weekly (Sonnet only)', used: 47, max: 100, resetsAt: ahead(6 * DAY + 23 * HOUR) },
            // Next year's, which the day names with its year.
            {
              id: 'annual',
              label: 'Annual credits',
              used: 12,
              max: 100,
              resetsAt: new Date(new Date().getFullYear() + 1, 0, 3, 9, 0).toISOString(),
            },
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
      id: 'codex', name: 'Codex', kind: 'subscription', family: 'codex', state: 'signed-out',
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
        // Signed in and reachable from the backend: its active account can be tested.
        accounts: [
          { id: 'g1', label: 'alex@example.com', plan: 'XPremium', active: true, quota: [] },
          { id: 'g2', label: 'alex@work.example', plan: '', active: false, quota: [] },
        ],
        models: [
          model({ id: 'grok-4-6', name: 'Grok 4.6', contextWindow: 256_000, outputLimit: 64_000, tools: true }),
        ],
      }
    ),
    provider({
      id: 'anthropic', name: 'Anthropic', kind: 'api_key', family: 'anthropic', vendorId: 'anthropic',
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
      id: 'openai', name: 'OpenAI', kind: 'api_key', family: 'openai', vendorId: 'openai',
      baseUrl: 'https://api.openai.com/v1', wireApi: 'openai-responses', apiKey: 'sk-proj-91ce4a7b2d8f6e1c3a5b9d7f',
      state: 'error', detail: '401 · Incorrect API key provided', catalogFetched: '3 days ago',
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
      id: 'kimi', name: 'Kimi', kind: 'api_key', family: 'anthropic', vendorId: 'moonshotai',
      baseUrl: 'https://api.moonshot.cn/anthropic', wireApi: 'anthropic-messages', apiKey: 'example-not-a-real-key', testedIn: '412 ms',
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
      id: 'vertex', name: 'Google Vertex', kind: 'api_key', family: 'google', vendorId: 'google-vertex',
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

/**
 * Entries that are not complete yet (`models.md` § A provider is in the list
 * from the start), one lacking each part and one lacking two: a vendor entry
 * without its key, a bare endpoint with a key and no model, a bare endpoint
 * with neither, and a subscription nobody has signed in to.
 */
export function incompleteProviders(): MockProvider[] {
  return [
    provider({
      id: 'codex-new', name: 'Codex', kind: 'subscription', family: 'codex', state: 'setup', missing: ['account'],
      autoRefreshUsage: false,
    }),
    provider({
      id: 'google', name: 'Google', kind: 'api_key', family: 'google', vendorId: 'google',
      wireApi: 'google-generative', state: 'setup', missing: ['apiKey'], modelSource: 'catalog',
      models: [
        model({ id: 'gemini-3.8-flash', name: 'Gemini 3.8 Flash', contextWindow: 1_000_000, outputLimit: 65_536, tools: true }),
        model({ id: 'gemini-3.8-pro', name: 'Gemini 3.8 Pro', contextWindow: 1_000_000, outputLimit: 65_536, tools: true }),
      ],
    }),
    provider({
      id: 'lab', name: 'Lab Server', kind: 'api_key', family: 'openai', baseUrl: 'http://lab.example:8000/v1',
      state: 'setup', missing: ['models'], modelSource: 'manual', configured: true, keyConfigured: true,
    }),
    provider({
      id: 'endpoint', name: 'OpenAI Responses API', kind: 'api_key', family: 'openai', wireApi: 'openai-responses',
      state: 'setup', missing: ['apiKey', 'models'], modelSource: 'manual',
    }),
  ]
}

/** What an entry still lacks, from what it holds: its key, its first model, its sign-in. */
export function missingParts(entry: SettingsProviderEntry): SettingsSetupPart[] {
  if (entry.kind === 'subscription') {
    return entry.accounts.length ? [] : ['account']
  }
  return [
    ...(entry.keyConfigured ? [] : ['apiKey' as const]),
    ...(entry.models.length ? [] : ['models' as const]),
  ]
}

/** The models.dev models the Add Model specimens search, as the backend's search answers. */
export const mockModelMatches: SettingsModelMatch[] = [
  ['google', 'gemini-3.8-flash', 'Gemini 3.8 Flash', 1_000_000, 65_536, ['low', 'medium', 'high']],
  ['google', 'gemini-3.8-flash-lite', 'Gemini 3.8 Flash Lite', 1_000_000, 65_536, ['low', 'medium']],
  ['google', 'gemini-3.5-flash', 'Gemini 3.5 Flash', 1_000_000, 65_536, ['low', 'medium', 'high']],
  ['google', 'gemini-3.8-pro', 'Gemini 3.8 Pro', 1_000_000, 65_536, ['low', 'medium', 'high']],
  ['openai', 'gpt-5.5', 'GPT-5.5', 400_000, 128_000, ['low', 'medium', 'high']],
  ['openai', 'gpt-5.5-mini', 'GPT-5.5 Mini', 400_000, 128_000, ['low', 'medium']],
  ['anthropic', 'claude-opus-4-7', 'Claude Opus 4.7', 1_000_000, 64_000, ['low', 'medium', 'high', 'max']],
  ['deepseek', 'deepseek-v4-pro', 'DeepSeek V4 Pro', 512_000, 64_000, []],
].map(([vendor, id, name, contextWindow, outputLimit, efforts]) => ({
  ref: `${vendor}/${id}`,
  vendor: vendor as string,
  model: {
    id: id as string,
    name: name as string,
    contextWindow: contextWindow as number,
    outputLimit: outputLimit as number,
    efforts: efforts as string[],
    extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.pdf'],
    fastTier: null,
  },
}))

/**
 * The backend's search stand-in: a match holds every word of the query in its
 * name or id, in any case and order, and the answer takes a beat, as a request
 * does; an aborted request answers with the abort, as `fetch` does.
 */
export function searchMockModels(query: string, signal: AbortSignal): Promise<SettingsModelMatch[]> {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => {
      resolve(
        mockModelMatches
          .filter((match) => words.every((word) => `${match.model.name} ${match.model.id}`.toLowerCase().includes(word)))
          .slice(0, 20),
      )
    }, 120)
    signal.addEventListener('abort', () => {
      window.clearTimeout(timer)
      reject(new DOMException('Aborted', 'AbortError'))
    })
  })
}

/** The paired devices the devices page lists. */
/** The projects on the paired devices, which a revoked device takes with it. */
function galleryDeviceProjects(): { deviceId: string; path: string }[] {
  return [
    { deviceId: 'mac', path: '/Users/alex/Projects/shop' },
    { deviceId: 'mac', path: '/Users/alex/Projects/notes' },
  ]
}

export function createSettingsState() {
  return reactive({
    quotaRefreshCache: createQuotaRefreshCache(),
    general: {
      language: 'English',
      theme: 'system' as 'light' | 'dark' | 'system',
      fontSize: DEFAULT_TRANSCRIPT_TEXT_SIZE,
    },
    notifications: {
      enabled: false,
      turnFinishes: true,
      turnFails: true,
      needsPermission: true,
      /** What the browser allows the site, as the product reads it. */
      permission: 'default' as NotificationPermission,
      /** What the stand-in browser answers when it is asked. */
      answer: 'granted' as 'granted' | 'denied',
    },
    account: {
      name: 'Alex',
      email: 'alex@example.com',
      passwordChanged: '3 months ago',
      passkeys: galleryPasskeys(),
      sessions: gallerySessions(),
    },
    /** The Users page as the master sees it. */
    users: createUsersState(('alex')),
    providers: mockProviders(),
    selectedProviderId: null as string | null,
    providerDetailOpen: false,
    plugins: [
      { id: 'file', name: 'Demi File Commands', description: 'Reads, writes, edits and searches the conversation’s files with demi file.', enabled: false },
      {
        id: 'browser',
        name: 'Browser',
        description: 'Gives the agent a browser on the conversation’s host, which it drives with demi browser and you watch and use in the work panel.',
        enabled: true,
      },
      {
        id: 'skills',
        name: 'Skills',
        description: 'Teaches the agent workflows with skills from Git repositories you add with demi skills, and from your repository.',
        enabled: true,
      },
      {
        id: 'web',
        name: 'Web',
        description: 'Searches the web with Exa and reads pages from the conversation’s Host with demi web.',
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
    credentials: createCredentialState(),
    instructions: DEMO_PERSONAL_INSTRUCTIONS,
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
            { name: 'vercel-composition-patterns', description: 'When to split a component and when to leave it.', warnings: ['The description is longer than 1,024 characters'], enabled: true, disableModelInvocation: false },
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
          origin: 'https://github.com/alex/commit',
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
    web: { account: null, key: null, inUse: 'anonymous' } as WebState,
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
    deviceProjects: galleryDeviceProjects(),
    // The product's shortcuts with their own keys: the gallery changes them as the product does.
    keys: APP_SHORTCUTS.map((shortcut): SettingsKeyBinding => ({ ...shortcut })),
    data: {
      telemetry: true,
    },
  })
}

export type SettingsState = ReturnType<typeof createSettingsState>
