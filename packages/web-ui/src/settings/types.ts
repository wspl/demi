/** Presentation models for the settings surfaces. Hosts map their own state onto these. */
import type { ShownError } from '../infra/errors'
import { VIDEO_FILE_EXTENSIONS } from '@demicodes/protocol'
import type { Component } from 'vue'
import type { ModelSettings } from '../agent/model-selection'
import type { TagTone } from '../ui/Tag.vue'
import type { SentenceText, TitleText } from '../ui/ui-text'
import type { DeviceState } from '../devices/state'
import type { DirectStatus } from '../devices/direct'
import type { DeviceReport } from '../devices/report'
import type { DeviceStart } from '../devices/installation'
import type { CredentialFieldView, CredentialKeep } from '../permissions/types'
import type { HandoffHost } from '../auth/handoff'
import type { VerificationHost } from '../auth/verification'

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

/**
 * The release's third-party notices as Settings › About › Acknowledgements
 * shows them. `missing`: the page's build includes none, as a development
 * build does; a release always does.
 */
export type AcknowledgementsState =
  | { status: 'loading' }
  | { status: 'ready'; text: string }
  | { status: 'missing' }
  | { status: 'failed'; error: ShownError }

export interface SettingsAccountInfo {
  /** The account id, which the avatar's tone follows. */
  id: string
  email?: string
  name: string
}

/** A paired device as the Devices page lists it, with what its runner last reported. */
export interface SettingsDevice extends DeviceReport {
  id: string
  name: string
  state: DeviceState
  /** When the host was last seen, as an ISO 8601 timestamp; shown while it is offline. */
  seen?: string
  /** How to start its runner again; its page's header gives it while it is offline. */
  start?: DeviceStart | null
  /** The device's home directory, which a location under it is written from as `~`. */
  home?: string | null
  /** When it was paired, as an ISO 8601 timestamp. */
  pairedAt: string
  /** How this page reaches it (`direct-channel.md` § What the user sees). */
  direct: DirectStatus
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

/** The protocols a compatible API can speak, the ones Add Provider offers (`product.md` § Models & providers). */
export const COMPATIBLE_WIRE_APIS: readonly SettingsWireApi[] = [
  'anthropic-messages',
  'openai-chat',
  'openai-responses',
]

/** What Add Provider asks for a compatible API once its protocol is chosen: the endpoint is required, the rest may come later. */
export interface SettingsCompatibleDraft {
  wireApi: SettingsWireApi
  label: string
  baseUrl: string
  /** Empty to add the key later. */
  apiKey: string
}

/** A models.dev vendor an API key can be added for. */
export interface SettingsVendor {
  id: string
  name: string
  wireApi: SettingsWireApi
  /** Null when the runtime knows the vendor's endpoint itself. */
  baseUrl: string | null
  /** The vendor mark (`vendorLogo`); null falls back to an initial. */
  logo: string | null
}

/**
 * `setup` is an entry that is not complete yet (`models.md` § A provider is
 * in the list from the start): a plain wait, never a failure. `error` and
 * `unreachable` are failures, which a refused key, an endpoint that did not
 * answer or a catalog that could not be read cause.
 */
export type SettingsProviderState =
  'ready' | 'setup' | 'error' | 'unreachable' | 'signed-out' | 'disabled'

/** A model models.dev lists, which the Add Model dialog offers to fill a model from. */
export interface SettingsModelMatch {
  /** `<vendor>/<model id>`, which names the match. */
  ref: string
  /** The models.dev vendor the match is filled from. */
  vendor: string
  /** The whole model as the entry's vendor names it; every field stays editable. */
  model: SettingsModelDraft
}

/**
 * Asks for the matches of what the user typed; the host answers from the
 * backend. A request the dialog no longer needs is aborted through `signal`.
 */
export type SettingsModelSearch = (query: string, signal: AbortSignal) => Promise<SettingsModelMatch[]>

/** A model a provider offers, as the page lists and toggles it. */
export interface SettingsProviderModel extends SettingsModelDraft {
  enabled: boolean
}

export interface SettingsQuotaWindow {
  id: string
  label: SentenceText
  used: number
  max: number
  /** When the window resets, as the vendor reported it; null when unknown. */
  resetsAt: string | null
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
  /** models.dev vendor; null for a compatible API and for a family's own vendor. */
  vendorId: string | null
  /**
   * A compatible API (`product.md` § Models & providers): its endpoint and
   * protocol are the user's to edit and its models are the ones the user
   * adds. Any other API-key entry is a vendor's: its endpoint is shown, never
   * edited. Fixed with the entry, so editing the endpoint never changes it.
   */
  compatible: boolean
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
  /** Where the model list comes from: the vendor catalog, or ids the user typed (always, for a compatible API). */
  modelSource: 'catalog' | 'manual'
  catalogFetched: string | null
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
  /** The vendor mark (`providerLogo`); null falls back to an initial. */
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
  /** Why the last save was refused, shown in the dialog. */
  error: ShownError | null
}

/**
 * What a sign-in waits for before it has a link or a code to show
 * (`claude-code.md` § Accounts and sign-in): the vendor, the user's Cloud,
 * the command-line tool's install on it, or its start.
 */
export type ProviderLoginStage = 'preparing' | 'waiting_for_cloud' | 'installing' | 'starting' | 'waiting_for_user'

export type ProviderLoginPhase =
  | { kind: 'starting'; stage: ProviderLoginStage }
  | {
      kind: 'device'
      url: string
      code: string
      expiresIn?: string
    }
  | {
      /** The vendor's own sign-in: the user signs in at `url` and pastes back the code the page shows. */
      kind: 'code'
      url: string
      expiresIn?: string
      /** The pasted code is with the sign-in, which is finishing. */
      submitted: boolean
      /** Why the sign-in refused the last code, in the vendor's words; it waits for another. */
      error?: SentenceText
    }
  | {
      kind: 'done'
      /** The account this sign-in added. */
      account: string
      /** Whether requests now go out on it: a first account is, one added beside another is not. */
      active: boolean
    }
  | ({ kind: 'failed' } & ShownError)

export type SettingsProviderOperation =
  | { kind: 'saving' | 'testing' | 'refreshing' | 'removing' | 'cli' }
  | { kind: 'account'; accountId: string; action: 'remove' | 'test' }

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
  /** The text added to a child's system prompt after Demi's own; null for none. */
  instructions: string | null
  /** Whether the profile's children may spawn children of their own. */
  canSpawn: boolean
  enabled: boolean
}

/** What the profile editor saves: a new profile starts enabled. */
export type SettingsSubagentDraft = Omit<SettingsSubagentProfile, 'id' | 'enabled'>

/** A credential as Settings › Credentials lists it (`credentials.md` § A credential), its values left out. */
export interface SettingsCredential {
  id: string
  name: string
  /** The reason it was asked for; empty for none. */
  note: string
  fields: SettingsCredentialField[]
  /** When it is removed; null while it is kept always. */
  keepUntil: string | null
  /** When an agent last read it; null before. */
  lastUsedAt: string | null
}

/** A field as the list shows it: a visible field with its value, a secret one without, which only the reveal reads. */
export interface SettingsCredentialField extends CredentialFieldView {
  /** A visible field's value; null for a secret one. */
  value: string | null
}

/** A field as the editor saves it: a new field carries its value, an existing one its value only when the user typed one. */
export interface SettingsCredentialDraftField extends CredentialFieldView {
  /** The new value; null keeps the saved one, which only an existing field may. */
  value: string | null
}

/** What the credential editor saves. */
export interface SettingsCredentialDraft {
  name: string
  note: string
  /** How long to keep it from now; null leaves a saved credential's time as it is. */
  keep: CredentialKeep | null
  fields: SettingsCredentialDraftField[]
}

/** A passkey of the account, as Settings › Account lists it (`product.md` § User system). */
export interface SettingsPasskey {
  id: string
  /** The password manager's name, the browser and system that added it, or the user's own. */
  name: string
  /** Backed up by a password manager that syncs it; otherwise on one device only. */
  synced: boolean
  createdAt: string
  lastUsedAt: string | null
}

/**
 * How adding a passkey with the account's password ended: added; refused for
 * the password, said under its field; the browser's prompt dismissed, which
 * says nothing; the browser's password manager holds a passkey of the
 * account already, which `message` says and which is no failure; or
 * failed, said under the dialog's fields.
 */
export type PasskeyAddOutcome =
  | { kind: 'added' }
  | { kind: 'wrongPassword'; message: string }
  | { kind: 'dismissed' }
  | { kind: 'held'; message: string }
  | ({ kind: 'failed' } & ShownError)

/**
 * How adding a passkey without the password ended: as with it, or the
 * backend wants the session confirmed first, as it does five minutes after
 * the last sign-in.
 */
export type PasskeyOfferOutcome = PasskeyAddOutcome | { kind: 'needsPassword' }

/** The steps the passkey offer's wizard has, which it names its own step by (`PasskeyOfferSteps`). */
export type PasskeyOfferStep = 'ask' | 'password' | 'held'

/** The account's passkeys and what the host does with them, for Settings › Account. */
export interface SettingsPasskeyHost {
  passkeys: readonly SettingsPasskey[]
  add: (password: string) => Promise<PasskeyAddOutcome>
  /** Renames the passkey; a refusal the host says itself. */
  rename: (id: string, name: string) => Promise<void>
  /** Removes the passkey; a refusal the host says itself. */
  remove: (id: string) => Promise<void>
}

/** A browser signed in to the account, as Settings › Account lists it (`product.md` § User system). */
export interface SettingsSession {
  id: string
  /** The browser and system it signed in from, such as Safari on iPhone. */
  name: string
  createdAt: string
  lastActiveAt: string
  /** The browser the user is reading the list in. */
  current: boolean
  /** The administrator whose sign-in code signed it in, by name; null for a browser the user signed in. */
  issuedBy: string | null
}

/** The account's signed-in browsers and what the host does with them, for Settings › Account. */
export interface SettingsSessionHost {
  sessions: readonly SettingsSession[]
  /** Signs another browser out; a refusal the host says itself. */
  signOut: (id: string) => Promise<void>
}

/** An account's role (`product.md` § User system). */
export type UserRole = 'master' | 'admin' | 'user'

/** An account as Settings › Users lists it (`web-api.md` § Account API), its password left out. */
export interface SettingsUser {
  id: string
  nickname: string
  email: string
  role: UserRole
  disabled: boolean
  /** The owner is still to choose its own password: the account is new, or an administrator reset its password. */
  choosesPassword: boolean
  /** The latest activity of any of its sessions, as an ISO 8601 timestamp; null before it first signed in. */
  lastActiveAt: string | null
}

/** What Add User… asks: the server makes the first password. */
export interface SettingsNewUser {
  email: string
  role: Exclude<UserRole, 'master'>
}

/**
 * How adding an account ended: added, with the first password the server made
 * and the account's name; refused because an account has the address, said
 * under the dialog's email field; the administrator's own confirmation is
 * needed first (the dialog asks it and tries again); or failed, said under
 * its fields.
 */
export type UserAddOutcome =
  | { kind: 'added'; id: string; nickname: string; password: string }
  | { kind: 'emailTaken' }
  | { kind: 'needsVerification' }
  | ({ kind: 'failed' } & ShownError)

/** How making a new first password ended: made, the confirmation is needed first, or failed. */
export type NewPasswordOutcome =
  | { kind: 'made'; password: string }
  | { kind: 'needsVerification' }
  /** The host said why itself. */
  | { kind: 'failed' }

/**
 * The accounts and what the host does with them, for Settings › Users. The
 * page decides from `users` and `viewerId` which actions each row offers; the
 * host carries them out, and says itself a refusal it could not hide.
 */
export interface SettingsUsersHost extends VerificationHost {
  users: readonly SettingsUser[]
  /** The signed-in user's account: its row is marked You and has no menu. */
  viewerId: string
  add: (user: SettingsNewUser) => Promise<UserAddOutcome>
  /** Makes a new first password for the account, which stops its earlier one; Reset Password… and New Password. */
  newPassword: (id: string) => Promise<NewPasswordOutcome>
  setRole: (id: string, role: Exclude<UserRole, 'master'>) => Promise<void>
  signOutEverywhere: (id: string) => Promise<void>
  setDisabled: (id: string, disabled: boolean) => Promise<void>
  /** The sign-in codes of the account `id` (`product.md` § User system): good for ten minutes and once; the dialog asks the confirmation itself. */
  signInCode: (id: string) => HandoffHost
}

/**
 * How choosing the account's own password ended (`ChoosePasswordWizard`):
 * chosen, or failed, said in the step.
 */
export type PasswordChoiceOutcome = { kind: 'chosen' } | ({ kind: 'failed' } & ShownError)
