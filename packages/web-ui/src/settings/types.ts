/** Presentation models for the settings surfaces. Hosts map their own state onto these. */
import { VIDEO_FILE_EXTENSIONS } from '@demicodes/protocol'
import type { Component } from 'vue'
import type { ModelSettings } from '../agent/model-selection'
import type { TagTone } from '../ui/Tag.vue'
import type { SentenceText, TitleText } from '../ui/ui-text'
import type { DeviceStart } from '../devices/installation'
import type { DeviceState } from '../devices/state'

/** A section id. Hosts choose their own set; the built-in four cover the product today. */
export type SettingsTab = string

/**
 * A row's state, as one status label right after its name: Updating, Failed,
 * Update available. `detail` says more on hover, such as a failure's message.
 */
export interface SettingsRowStatus {
  label: SentenceText
  tone?: TagTone
  detail?: SentenceText
}

/**
 * One setting a section holds, which the rail filter finds by its label or
 * its keywords and opens with its row highlighted, as System Settings' search
 * does.
 */
export interface SettingsEntry {
  /** The label of its row on the section's page, which the highlight finds the row by. */
  label: SentenceText
  /** Other words a person may look for it by, such as "dark mode" for Theme. */
  keywords?: string[]
}

export interface SettingsNavItem {
  id: SettingsTab
  label: TitleText
  icon: Component
  /** What else the rail filter matches the section by, besides its settings. */
  keywords?: string[]
  /** The settings on the section's page that the rail filter finds one by one. */
  settings?: SettingsEntry[]
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: SentenceText
  /** It sets up a keyboard, so a touch phone, which has none, does not list it. */
  keyboard?: boolean
}

/** Sections grouped under a small caption, the way a long rail is read. */
export interface SettingsNavGroup {
  label?: TitleText
  items: SettingsNavItem[]
}

export interface SettingsAccountInfo {
  email?: string
  name: string
}

export interface SettingsDevice {
  id: string
  name: string
  state: DeviceState
  /** When the host was last seen, as an ISO 8601 timestamp; shown while it is offline. */
  seen?: string
  /** How to start its runner again; shown while it is offline. */
  start?: DeviceStart | null
  /**
   * The page's direct channel to the device (`direct-channel.md`):
   * `connected` while this page has one, `blocked` while this browser
   * blocks direct connections to devices on this computer and network.
   */
  direct?: 'connected' | 'blocked'
}

/** A plugin as the Plugins page lists it. */
export interface SettingsPlugin {
  id: string
  /** A feature's name, so title style: File Browser. */
  name: TitleText
  description: SentenceText
  enabled: boolean
}

/** An archived conversation as the Archived page lists it. */
export interface SettingsArchivedConversation {
  id: string
  title: string
  /** When it was archived or last active, formatted by the host. */
  detail?: string
}

/** A shortcut the keyboard page lists and rebinds. */
export interface SettingsKeyBinding {
  id: string
  action: string
  /** In the recorder's notation, e.g. `⇧⌘O`. */
  keys: string
}

/** A model as the settings edit it: what a custom endpoint needs to be usable. */
export interface SettingsModelDraft {
  id: string
  name: string
  contextWindow: number | null
  outputLimit: number | null
  /** Thinking efforts the composer offers; the first is the default. Empty means no control. */
  efforts: string[]
  /** Accepted attachment extensions, dot-prefixed. Empty means text only. */
  extensions: string[] | null
  fastTier: string | null
}

/** The API format an endpoint speaks. */
export type SettingsWireApi =
  | 'anthropic-messages'
  | 'openai-responses'
  | 'openai-chat'
  | 'google-generative'

export const WIRE_API_LABELS: Record<SettingsWireApi, string> = {
  'google-generative': 'Google Generative AI',
  'anthropic-messages': 'Anthropic Messages',
  'openai-responses': 'OpenAI Responses',
  'openai-chat': 'OpenAI Chat Completions',
}

/** A models.dev vendor an API key can be added for. */
export interface SettingsVendor {
  id: string
  name: string
  wireApi: SettingsWireApi
  /** Null when the runtime knows the vendor's endpoint itself. */
  baseUrl: string | null
  logo: string
}

/** `unconfigured` is a fresh entry that still needs its key or login. */
export type SettingsProviderState =
  'ready' | 'unconfigured' | 'error' | 'unreachable' | 'signed-out' | 'disabled'

/** A model a provider offers, as the page lists and toggles it. */
export interface SettingsProviderModel extends SettingsModelDraft {
  enabled: boolean
}

export interface SettingsQuotaWindow {
  id: string
  label: SentenceText
  used: number
  max: number
  resets: string | null
}

export type SettingsQuotaRefresh = 'automatic' | 'manual'

export interface SettingsProviderAccount {
  id: string
  label: string
  plan: string
  active: boolean
  /** Rate-window quotas, when the vendor exposes them. */
  quota: SettingsQuotaWindow[]
}

/** The CLI a process provider starts, which Demi installs and updates on the machines that run it. */
export interface SettingsProviderCli {
  /** The vendor's newest version, or why it could not be read. */
  newest: { version: string } | { error: string }
  /** The last install on the user's Cloud, which adding an account starts. */
  install: { state: 'installing' } | { state: 'installed' } | { state: 'failed'; message: string } | null
  /** The machines that could be asked now, and what each has; null when one did not answer. */
  machines: Array<{ id: string; name: string; versions: string[] | null }>
}

/**
 * A provider as the settings page shows and edits it. The page emits field changes and actions for the host: adding, removing, signing in, testing, refreshing, saving a model.
 */
export interface SettingsProviderEntry {
  id: string
  name: string
  kind: 'api_key' | 'subscription'
  /** models.dev vendor; null for a bare endpoint. */
  vendorId: string | null
  baseUrl: string
  wireApi: SettingsWireApi
  apiKey: string
  keyConfigured?: boolean
  configured?: boolean
  /**
   * The CLI its requests start on a machine, which Demi installs there, once
   * asked for; null while it is not known, absent for a provider whose
   * requests are HTTP.
   */
  cli?: SettingsProviderCli | null
  /** Where the model list comes from: the vendor catalog, or ids the user typed. */
  modelSource: 'catalog' | 'manual'
  catalogFetched: string | null
  stale: boolean
  state: SettingsProviderState
  /**
   * What went wrong, from the last test or request: the provider's own words,
   * passed through in full and never paraphrased. Never a note about a healthy
   * provider.
   */
  detail?: string
  /** How long the last successful test took, formatted by the host. */
  testedIn?: string
  testPassed?: boolean
  /** The model the last test asked: a plan can cover some models and refuse others. */
  testedWith?: string
  enabled: boolean
  models: SettingsProviderModel[]
  accounts: SettingsProviderAccount[]
  /** The vendor supports a free quota request when an account becomes visible. */
  autoRefreshUsage?: boolean
  /** The vendor mark; null falls back to an initial. */
  logo: string | null
}

export const THINKING_EFFORTS = [
  'minimal',
  'low',
  'medium',
  'high',
  'max',
] as const

export const EXTENSION_PRESETS: {
  id: string
  label: SentenceText
  extensions: string[]
}[] = [
  {
    id: 'images',
    label: 'Images',
    extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp'],
  },
  {
    id: 'videos',
    label: 'Videos',
    extensions: VIDEO_FILE_EXTENSIONS.map((extension) => `.${extension}`),
  },
  {
    id: 'documents',
    label: 'Documents',
    extensions: ['.pdf'],
  },
]

/** Retained by the host so a submitted model edit survives closing settings. */
export interface SettingsModelEditor {
  providerId: string
  mode: 'create' | 'edit' | 'view'
  model: SettingsModelDraft
  original: SettingsProviderModel | null
  open: boolean
  status: { kind: 'idle' | 'saving' } | { kind: 'failed'; message: string }
}

export type ProviderLoginPhase =
  | { kind: 'starting' }
  | {
      kind: 'device'
      url: string
      code: string
      expiresIn?: string
    }
  | {
      kind: 'token'
      /** The command that prints the token, run in the user's own terminal. */
      command: string
      /** Where to get the CLI when it is missing. */
      install: {
        label: TitleText
        url: string
      }
      /** What a token starts with, so a paste can be checked before it is sent. */
      prefix: string
    }
  | {
      kind: 'done'
      /** The account this sign-in added. */
      account: string
      /** Whether requests now go out on it: a first account is, one added beside another is not. */
      active: boolean
    }
  | {
      kind: 'failed'
      message: SentenceText
    }

export type SettingsProviderOperation =
  | { kind: 'saving' | 'testing' | 'refreshing' | 'removing' | 'cli' }
  | { kind: 'account'; accountId: string; action: 'activate' | 'remove' | 'test' }

/**
 * A subagent profile as the Subagent section lists and edits it
 * (`subagents.md` § Profiles). The settings carry its model as a
 * conversation's are carried, or null for the parent's.
 */
export interface SettingsSubagentProfile {
  id: string
  name: string
  /** When the agent should use the profile, which the model reads. */
  description: string
  /** The model a child infers with; null for its parent's. */
  model: ModelSettings | null
  /** The text that replaces a child's instructions; null for its parent's. */
  instructions: string | null
  /** Whether the profile's children may spawn children of their own. */
  canSpawn: boolean
  enabled: boolean
}

/** What the profile editor saves: a new profile starts enabled. */
export type SettingsSubagentDraft = Omit<SettingsSubagentProfile, 'id' | 'enabled'>
