import { t } from './locales/text'
/** Presentation models for the settings surfaces. Hosts map their own state onto these. */
import type { ShownError } from '../infra/errors'
import { VIDEO_FILE_EXTENSIONS, type QuotaWindowName } from '@demicodes/protocol'
import type { Component } from 'vue'
import type { ModelSettings } from '../agent/model-selection'
import type { TagTone } from '../ui/Tag.vue'
import type { SentenceText, TitleText } from '../ui/ui-text'
import type { AutostartUnavailable } from '../devices/autostart'
import type { DeviceMark } from '../devices/mark'
import type { DeviceState } from '../devices/state'
import type { DirectStatus } from '../devices/direct'
import type { DeviceReport } from '../devices/report'
import type { DeviceStart } from '../devices/installation'
import type { CredentialFieldView } from '../permissions/types'
import type { HandoffHost } from '../auth/handoff'
import type { VerificationHost } from '../auth/verification'

/**
 * The small model the user chose in Settings › Agent
 * (`crates/backend-providers/src/llm/small_model.rs`): model settings without a
 * tier, since small requests use the
 * default one; none for Automatic.
 */
export type SmallModelChoice = Pick<ModelSettings, 'providerId' | 'modelId' | 'thinkingEffort'>

/** A kind of media an entry may make (`media.md` § Which entries make which media), in the order its sections follow. */
export type SettingsMediaKind = 'image' | 'video' | 'audio'

export const SETTINGS_MEDIA_KINDS: readonly SettingsMediaKind[] = ['image', 'video', 'audio']

/** How an entry's API makes images: a vendor entry's is its vendor's, a compatible API's the one its user chose. */
export type SettingsImageShape = 'chatgpt' | 'grok' | 'openai-images' | 'gemini' | 'seedream' | 'minimax'

/** How an entry's API makes videos, as `SettingsImageShape` says of images. */
export type SettingsVideoShape = 'grok' | 'xai-videos' | 'veo' | 'openai-videos' | 'ark-tasks' | 'openrouter-videos' | 'hailuo'

/** How an entry's API makes audio, as `SettingsImageShape` says of images. */
export type SettingsAudioShape = 'openai-speech' | 'gemini' | 'elevenlabs' | 'seed' | 'minimax'

/** What an audio model makes (`audio.md` § Commands). */
export type SettingsAudioKind = 'speech' | 'music' | 'effect'

/** Each kind's shapes. */
export interface SettingsMediaShapes {
  image: SettingsImageShape
  video: SettingsVideoShape
  audio: SettingsAudioShape
}

/** A shape of any kind. */
export type SettingsMediaShape = SettingsMediaShapes[SettingsMediaKind]

/** The names of the shapes, the vendors' own formats, which no language translates. */
export const MEDIA_SHAPE_LABELS: Record<SettingsMediaShape, string> = {
  chatgpt: 'ChatGPT',
  grok: 'Grok',
  'openai-images': 'OpenAI Images',
  gemini: 'Gemini',
  'xai-videos': 'xAI Videos',
  veo: 'Veo',
  'openai-videos': 'OpenAI Videos',
  'ark-tasks': 'Ark Tasks',
  'openrouter-videos': 'OpenRouter Videos',
  'openai-speech': 'OpenAI Speech',
  elevenlabs: 'ElevenLabs',
  seedream: 'Seedream',
  seed: 'Seed speech and Seed Audio',
  minimax: 'MiniMax',
  hailuo: 'Hailuo',
}

/** The shapes a compatible API may choose for each kind, besides none, in the menu's order. */
export const COMPATIBLE_MEDIA_SHAPES: { readonly [K in SettingsMediaKind]: readonly SettingsMediaShapes[K][] } = {
  image: ['openai-images'],
  video: ['xai-videos', 'openai-videos', 'ark-tasks', 'openrouter-videos'],
  audio: ['openai-speech'],
}

/** A vendor's media model as an entry's section lists it, never edited. */
export interface SettingsMediaModel {
  id: string
  name: string
  /** `YYYY-MM-DD`; null when nobody states it. */
  released: string | null
  /** An audio model's kinds, which its row names; absent for another kind. */
  kinds?: SettingsAudioKind[]
}

/** How an entry makes one kind of media: its shape, null for none, and its switch. */
export interface SettingsEntryMedia<K extends SettingsMediaKind = SettingsMediaKind> {
  shape: SettingsMediaShapes[K] | null
  enabled: boolean
  /** A vendor entry's models of the kind, newest first; a compatible API's are its list's, kind and all. */
  models?: SettingsMediaModel[]
}

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

/** The groups the rail captions; each is worded in the page's language. */
export type SettingsGroupId = 'agent' | 'workspace'

/** Sections grouped under a small caption, the way a long rail is read. */
export interface SettingsNavGroup {
  /** The caption's id; the rail words it (`settingsGroupName`). None for the groups without a caption. */
  id?: SettingsGroupId
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
  /** How much of its next runner it has downloaded, while that download moves (`DeviceStatus`). */
  downloadPercent?: number | null
  /** When the host was last seen, as an ISO 8601 timestamp; shown while it is offline. */
  seen?: string
  /** How to start its runner again; its page's header gives it while it is offline. */
  start?: DeviceStart | null
  /**
   * Why it will not start its runner by itself after a restart, as its
   * runner last said; its page's header gives it. Null when it does.
   */
  autostartUnavailable?: AutostartUnavailable | null
  /** The device's home directory, which a location under it is written from as `~`. */
  home?: string | null
  /** When it was paired, as an ISO 8601 timestamp. */
  pairedAt: string
  /** How this page reaches it (`direct-channel.md` § What the user sees). */
  direct: DirectStatus
  /** Its mark, with the number it shows (`shownMark`); null draws the plain computer. */
  mark: DeviceMark | null
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
  /** Null for the placeholder, which shows as New conversation in the reader's language. */
  title: string | null
  /** When it was archived or last active, formatted by the host. */
  detail?: string
}

/** A shortcut the keyboard page lists and rebinds. */
export interface SettingsKeyBinding {
  id: string
  /** In the recorder's notation, e.g. `⇧⌘O`. */
  keys: string
}

/** A model as the settings edit it: what an entry's own list needs to use it. */
export interface SettingsModelDraft {
  /** What it makes: text, the model menu's, unless added as an image model of a compatible API; absent is text. An image model has only its ID and name. */
  kind?: 'text' | SettingsMediaKind
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

/**
 * Which account each agent of a subscription uses
 * (`crates/backend-providers/src/llm/rotation.rs`): the active one, the first
 * in the list that is not used up, or
 * the one with the most room.
 */
export type SettingsRotation = 'manual' | 'in_order' | 'balanced'

export const ROTATIONS: readonly SettingsRotation[] = ['manual', 'in_order', 'balanced']

export const ROTATION_LABELS: Record<SettingsRotation, TitleText> = {
  get manual() {
    return t('providers.rotation.manual')
  },
  get in_order() {
    return t('providers.rotation.inOrder')
  },
  get balanced() {
    return t('providers.rotation.balanced')
  },
}

/** What each rotation does, as the menu's row says it. */
export const ROTATION_HINTS: Record<SettingsRotation, SentenceText> = {
  get manual() {
    return t('providers.rotation.manualHint')
  },
  get in_order() {
    return t('providers.rotation.inOrderHint')
  },
  get balanced() {
    return t('providers.rotation.balancedHint')
  },
}

/** The protocols a compatible API can speak, which its page offers (`packages/web-ui/src/settings/SettingsProvidersPage.vue`). */
export const COMPATIBLE_WIRE_APIS: readonly SettingsWireApi[] = [
  'anthropic-messages',
  'openai-chat',
  'openai-responses',
]

/** A models.dev vendor an API key can be added for. */
export interface SettingsVendor {
  id: string
  name: string
  /** The protocol its entries speak; null for a vendor of media alone, such as ElevenLabs, which speaks its own. */
  wireApi: SettingsWireApi | null
  /** Null when the runtime knows the vendor's endpoint itself. */
  baseUrl: string | null
  /** The vendor mark (`vendorLogo`); null falls back to an initial. */
  logo: string | null
}

/**
 * `setup` is an entry that is not complete yet
 * (`crates/backend-user-shard/src/provider_settings.rs`): a plain wait, never a
 * failure. `error` and
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
  /** An audio model's kinds, as its host knows them, which its row names; absent for another kind. */
  kinds?: SettingsAudioKind[]
}

export interface SettingsQuotaWindow {
  id: string
  /** What names the window, which the page words in its language (`windowName`). */
  name: QuotaWindowName
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
  /** The vendor refused the account's sign-in: its row offers Sign In Again, and rotation passes over it. */
  signedOut?: boolean
  /** Rate-window quotas, when the vendor exposes them. */
  quota: SettingsQuotaWindow[]
}

/** The CLI a process provider starts, which Demi installs and updates on Claude Code's machine. */
export interface SettingsProviderCli {
  /** The vendor's newest version, or why it could not be read. */
  newest: { version: string } | { error: string }
  /** The last install on Claude Code's machine, which adding an account starts. */
  install: { state: 'installing' } | { state: 'installed' } | { state: 'failed'; message: string } | null
}

/**
 * A provider as the settings page shows and edits it. The page emits field changes and actions for the host: adding, removing, signing in, testing, refreshing, saving a model.
 */
export interface SettingsProviderEntry {
  id: string
  name: string
  kind: 'api_key' | 'subscription'
  /** The entry's vendor; null for a compatible API. */
  vendorId: string | null
  /**
   * A compatible API (`packages/web-ui/src/settings/SettingsProvidersPage.vue`): its protocol,
   * endpoint and optional key are the user's to set at any time, and its
   * models are the ones the user adds. Any other API-key entry is a vendor's:
   * its endpoint is shown, never edited. Fixed with the entry.
   */
  compatible: boolean
  /**
   * Whether it makes text: false for a vendor of media alone, such as
   * ElevenLabs (`packages/web/src/state/catalog.ts`), whose page has no
   * Models and whose key Test Connection checks through its media models.
   * Absent means it does.
   */
  makesText?: boolean
  /** Where its requests go; empty for a compatible API not given an endpoint yet. */
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
  /**
   * The port of the user's device the last test named and the device has not
   * exposed, which the test's result offers to expose (`port-exposure.md`).
   */
  notExposed?: { deviceId: string; device: string; port: number }
  enabled: boolean
  models: SettingsProviderModel[]
  /** In the order In Order follows, which the user changes by dragging. */
  accounts: SettingsProviderAccount[]
  /** A subscription's rotation; absent for an API key. */
  rotation?: SettingsRotation
  /**
   * How it makes each kind of media (`media.md` § Which entries make which
   * media): a kind with a shape has its switch, such as Image Generation, on its page.
   */
  media: { [K in SettingsMediaKind]: SettingsEntryMedia<K> }
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
    get label() {
      return t('presets.images')
    },
    extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp'],
  },
  {
    id: 'videos',
    get label() {
      return t('presets.videos')
    },
    extensions: VIDEO_FILE_EXTENSIONS.map((extension) => `.${extension}`),
  },
  {
    id: 'documents',
    get label() {
      return t('presets.documents')
    },
    extensions: ['.pdf'],
  },
]

/** Retained by the host so a submitted model edit survives closing settings. */
export interface SettingsModelEditor {
  providerId: string
  /** Edit Model for a model of the entry's own list; the facts of a catalog's model. */
  mode: 'edit' | 'view'
  model: SettingsModelDraft
  original: SettingsProviderModel | null
  open: boolean
  /** Why the last save was refused, shown in the dialog. */
  error: ShownError | null
}

/**
 * What a sign-in waits for before it has a link or a code to show
 * (`crates/provider-claude-code/src/account.rs`): the vendor, Claude Code's machine,
 * the command-line tool's install on it, or its start.
 */
export type ProviderLoginStage = 'preparing' | 'waiting_for_machine' | 'installing' | 'starting' | 'waiting_for_user'

export type ProviderLoginPhase =
  | { kind: 'starting'; stage: ProviderLoginStage }
  | {
      kind: 'device'
      url: string
      code: string
      /** When the sign-in ends unless the user finishes it, on the page's clock, which the dialog counts down to. */
      expiresAt?: number
    }
  | {
      /** The vendor's own sign-in: the user signs in at `url` and pastes back the code the page shows. */
      kind: 'code'
      url: string
      /** As a device sign-in's. */
      expiresAt?: number
      /** The pasted code is with the sign-in, which is finishing. */
      submitted: boolean
      /**
       * The number the sign-in gave the code this page pasted last, once it
       * answered; a new one empties the field, which nothing else does while
       * the sign-in waits for a code, another page's paste included.
       */
      taken?: number
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
 * (`crates/agent-server/src/server/tree/profiles.rs`). The settings carry its model as a
 * conversation's are carried, or null for the parent's.
 */
export interface SettingsSubagentProfile {
  id: string
  name: string
  /** When the agent should use the profile, which the model reads. */
  description: string
  /** The model a child infers with; null for its parent's. */
  model: ModelSettings | null
  /**
   * The profile's own Context in tokens, its model's full window or a
   * limit, in place of the user's limit; null for the user's, and always
   * with the parent's model.
   */
  contextLimit: number | null
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

/**
 * How reading a credential's values for the eye ended: its values by field
 * name; the user's confirmation is needed first (the page asks it and reads
 * again); or failed, which the host has told the user.
 */
export type CredentialRevealOutcome =
  | { kind: 'revealed'; values: Record<string, string> }
  | { kind: 'needsVerification' }
  | { kind: 'failed' }

/** What the credential editor saves. */
export interface SettingsCredentialDraft {
  name: string
  note: string
  /** `always` for a new credential, or one the user chose Keep Always for; null leaves a saved credential's time as it is. */
  keep: 'always' | null
  fields: SettingsCredentialDraftField[]
}

/** A passkey of the account, as Settings › Account lists it. */
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

/** A browser signed in to the account, as Settings › Account lists it. */
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

/** An account's role. */
export type UserRole = 'master' | 'admin' | 'user'

/** An account as Settings › Users lists it, its password left out. */
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
  /** The size of its Cloud disks, which its Cloud's next start takes. */
  cloudStorage: SettingsCloudStorage
}

/** The size of an account's Cloud disks in whole GB, each from 1 to 1024. */
export interface SettingsCloudStorage {
  systemGb: number
  homeGb: number
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
  /** Sets the size of the account's Cloud disks, the viewer's own included. */
  setCloudStorage: (id: string, storage: SettingsCloudStorage) => Promise<void>
  /** The sign-in codes of the account `id`: good for ten minutes and once; the dialog asks the confirmation itself. */
  signInCode: (id: string) => HandoffHost
}

/**
 * How choosing the account's own password ended (`ChoosePasswordWizard`):
 * chosen, or failed, said in the step.
 */
export type PasswordChoiceOutcome = { kind: 'chosen' } | ({ kind: 'failed' } & ShownError)
