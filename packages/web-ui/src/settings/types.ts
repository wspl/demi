/** Presentation models for the settings surfaces. Hosts map their own state onto these. */
import { VIDEO_FILE_EXTENSIONS } from '@demicodes/core'
import type { Component } from 'vue'

/** A section id. Hosts choose their own set; the built-in four cover the product today. */
export type SettingsTab = string

export interface SettingsNavItem {
  id: SettingsTab
  label: string
  icon: Component
  /** What the rail filter also matches: names of settings the section holds. */
  keywords?: string[]
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
}

/** Sections grouped under a small caption, the way a long rail is read. */
export interface SettingsNavGroup {
  label?: string
  items: SettingsNavItem[]
}

export interface SettingsAccountInfo {
  email?: string
  name: string
}

export interface SettingsDevice {
  id: string
  name: string
  online: boolean
  /** When the host last connected; shown while it is offline. */
  seen?: string
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
  /** In the recorder's notation, e.g. `⌘⇧O`. */
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
  label: string
  used: number
  max: number
  resets: string | null
}

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
  /** The version the entry is held at instead of following the newest. */
  held: string | null
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
  /** Its requests run as a process on a machine, started from a CLI Demi installs there. */
  runsOnHost?: boolean
  /** That CLI, once asked for; null while it is not known. */
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
  /** The vendor mark; null falls back to an initial. */
  logo: string | null
}

export type SettingsMcpTransport = 'stdio' | 'http'
export type SettingsMcpState = 'connected' | 'auth' | 'crashed' | 'disabled'

/** A tool server as the settings page lists it. Tools are names only; there is no per-tool switch. */
export interface SettingsMcpServer {
  id: string
  name: string
  transport: SettingsMcpTransport
  target: string
  state: SettingsMcpState
  enabled: boolean
  /** Why it is not connected, shown on the status. */
  detail?: string
  tools: string[]
}

export interface SettingsMcpDraft {
  transport: SettingsMcpTransport
  name: string
  target: string
}

export type SettingsSkillSourceState = 'ready' | 'updating' | 'error'

/** One skill inside a source. Off keeps the files but hides them from the agent. */
export interface SettingsSkill {
  id: string
  name: string
  description: string
  enabled: boolean
}

/** A git repository that produced one or more skills. */
export interface SettingsSkillSource {
  id: string
  name: string
  origin: string
  state: SettingsSkillSourceState
  detail?: string
  skills: SettingsSkill[]
}

export interface SettingsSkillDraft {
  origin: string
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
  label: string
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
        label: string
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
      message: string
    }

export type SettingsProviderOperation =
  | { kind: 'saving' | 'testing' | 'refreshing' | 'removing' | 'cli' }
  | { kind: 'account'; accountId: string; action: 'activate' | 'remove' | 'test' | 'usage' }
