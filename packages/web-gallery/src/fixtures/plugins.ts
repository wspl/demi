import { reactive } from 'vue'
import { createPinia } from 'pinia'
import { httpStatusLine } from '@demicodes/utils'
import { BrowserTabsError } from '@demicodes/plugin-browser/live/tabs'
import type { OpenUserStream } from '@demicodes/web-ui/plugins/streams'
import type { HostArtifact, HostDownload } from '@demicodes/web-ui/devices/installed'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { workspaceName } from '@demicodes/web-ui/workspaces/workspaces'
import {
  PluginCallError,
  type AnyPluginPage,
  type ConversationFileService,
  type IntentService,
  type PageHost,
  type StateFeed,
  type PageDevice,
} from '@demicodes/web-ui/plugins/page'
import { bindTabSchema, navigateTabSchema, stopTabSchema, tabHistorySchema } from '@demicodes/plugin-browser'
import {
  addSourceSchema,
  setEnabledSchema,
  setFetcherSchema,
  setSourceEnabledSchema,
  sourceCallSchema,
  type SkillsState,
  type SourceState,
} from '@demicodes/plugin-skills'
import {
  FEATURES as WEB_FEATURES,
  removeKeySchema,
  saveKeySchema,
  signInSchema,
  signOutSchema,
  type WebState,
} from '@demicodes/plugin-web'
import { productWould } from '../product-would'
import { galleryWorkspaces } from './workspaces'
import { galleryDevices } from './devices'
import type { GalleryBrowser } from './live-browser'
import type { GalleryBrowserPlugin } from './panel'

/**
 * The plugins as the gallery's specimens answer them: each over the specimen's own state, so every
 * control of a plugin's page acts on it the way the backend's plugin would, after a beat where the
 * plugin would reach a Host or git.
 */
export interface GalleryPlugin {
  /** The plugin's user state, read reactively. */
  state?(): unknown
  /** Whether the specimen's user has each of its features on, which a switch changes at once. */
  features?: Record<string, boolean>
  /** Follows the plugin's state of a conversation, as the product follows it by revision. */
  followState?(conversation: string): StateFeed
  call(method: string, params: object, conversation: string | null): Promise<unknown>
  /** Its user streams, by name. */
  streams?: Record<string, OpenUserStream>
  /** What the specimen's Host holds of its packages, read reactively. */
  installed?(): readonly HostArtifact[]
  /** What the specimen's Host downloads of its packages now, read reactively. */
  downloading?(): readonly HostDownload[]
  /** Where its reply block `tag` of `id` loads, as the backend's route would serve it. */
  blockDocument?(tag: string, id: string): string
}

/**
 * The shell's side of a specimen: its conversation's files, its panel and
 * its intents. A specimen without a panel gives none, and a control that
 * reaches one says what the product would do.
 */
export interface GalleryShell {
  files?: ConversationFileService
  panel?: PageHost['panel']
  intents?: IntentService
  /** Whether the specimen's Host is a Cloud that is starting; never, unless the specimen says. */
  hostStarting?(): boolean
  /** What the specimen says the product would do with a page it opens in a new tab. */
  openLink?: PageHost['openLink']
  /** Whether the user has `plugin` on, read reactively; every plugin is, unless the specimen says. */
  enabled?(plugin: string): boolean
  /** The workspace of the specimen's tab; the gallery's first without one. */
  workspace?: string
  /** The user's paired devices; the gallery's without them. */
  devices?: readonly PageDevice[]
  /** The pages whose reply blocks the specimen's replies draw; none without them. */
  pages?: readonly AnyPluginPage[]
}

/**
 * A page host over `plugins` and the specimen's `shell`: a plugin it lacks
 * refuses as the backend would, and so does one the specimen has turned off,
 * its calls with `plugin_disabled` and its streams closed with it. Each host
 * holds a Pinia of its own, so a specimen's plugin stores are its alone.
 */
export function galleryPageHost(plugins: Record<string, GalleryPlugin>, shell: GalleryShell = {}): PageHost {
  const enabled = (plugin: string) => shell.enabled?.(plugin) ?? true
  const fixture = (plugin: string): GalleryPlugin => {
    const found = plugins[plugin]
    if (!found) {
      throw new PluginCallError('unknown_plugin', `No plugin "${plugin}"`)
    }
    if (!enabled(plugin)) {
      // As the backend's plugin call route answers it.
      throw new PluginCallError('plugin_disabled', `The plugin "${plugin}" is turned off`, {
        type: 'status',
        status: '409 Conflict',
      })
    }
    return found
  }
  return {
    pages: () => shell.pages ?? [],
    blockDocument: (plugin, tag, id) => plugins[plugin]?.blockDocument?.(tag, id) ?? 'about:blank',
    userState: (plugin) => (enabled(plugin) ? plugins[plugin]?.state?.() : undefined),
    enabled,
    features: (plugin) => plugins[plugin]?.features ?? {},
    setFeature: async (plugin, feature, on) => {
      const features = fixture(plugin).features
      if (!features || !(feature in features)) {
        throw new PluginCallError('unknown_plugin', `The plugin "${plugin}" has no feature "${feature}"`)
      }
      features[feature] = on
    },
    followState: (plugin, conversation) => {
      const follow = fixture(plugin).followState
      if (!follow) {
        throw new PluginCallError('unknown_plugin', `The plugin "${plugin}" has no conversation state`)
      }
      return follow(conversation)
    },
    call: async (plugin, method, params, conversation) => fixture(plugin).call(method, params, conversation),
    stream: (name) => {
      for (const [id, plugin] of Object.entries(plugins)) {
        const stream = plugin.streams?.[name]
        if (!stream) {
          continue
        }
        return (handlers, options) => {
          if (enabled(id)) {
            return stream(handlers, options)
          }
          // As the backend closes the stream of a plugin that is off, after the page has its stream.
          queueMicrotask(() => handlers.closed('plugin_disabled'))
          return { send: () => {}, watched: () => {}, close: () => {} }
        }
      }
      throw new PluginCallError('unknown_stream', `No user stream "${name}"`)
    },
    installed: (plugin) => plugins[plugin]?.installed?.() ?? [],
    downloading: (plugin) => plugins[plugin]?.downloading?.() ?? [],
    hostStarting: () => shell.hostStarting?.() ?? false,
    files: () => {
      if (!shell.files) {
        throw new Error('The specimen shows no conversation files')
      }
      return shell.files
    },
    intents: shell.intents ?? {
      open: (_conversation, request) => productWould(`Open the ${request.intent} in the Work Panel`),
      canOpen: () => true,
    },
    panel: shell.panel ?? {
      tabs: () => [],
      select: (_conversation, kind) => productWould(`Select the ${kind} Tab in the Work Panel`),
      // A specimen without a panel has no history of one.
      canGo: () => false,
      go: () => {},
    },
    workspaces: {
      list: () => galleryWorkspaces.list.map((workspace) => ({ id: workspace.id, name: workspaceName(workspace) })),
      current: () => shell.workspace ?? galleryWorkspaces.list[0]?.id ?? null,
    },
    devices: {
      list: () =>
        (shell.devices ?? galleryDevices().map((demo) => demo.device)).map((device) => ({
          id: device.id,
          name: device.name,
          state: device.state,
          mark: device.mark,
          downloadPercent: device.downloadPercent ?? null,
        })),
    },
    openSettings: (section) => productWould(`Open ${section} Settings`),
    openLink: shell.openLink ?? (async (url) => {
      await url
      productWould('Open the Page in a New Tab')
    }),
    overlays: appOverlayStore,
    pinia: createPinia(),
  }
}

/** The status the backend answers a plugin's failure with, which a specimen's failure without a reason stands for. */
export const SERVER_FAILED = httpStatusLine(500)

function unknownMethod(method: string): never {
  throw new PluginCallError('unknown_method', `No method "${method}"`)
}

/**
 * A beat, as long as the backend's plugin would take. Not cancelled with
 * the specimen: what follows it changes only the specimen's own state.
 */
function beat(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/**
 * The conversation browser's plugin over the gallery's `browser`: its tab
 * list, its tab methods, and `panel`, its part in the panel's tabs.
 */
export function browserPlugin(browser: GalleryBrowser, panel: GalleryBrowserPlugin): GalleryPlugin {
  async function call(method: string, params: object): Promise<unknown> {
    switch (method) {
      case 'bind':
        return { tab: await panel.bind(bindTabSchema.parse(params).panelTab) }
      case 'navigate': {
        const { tab, url } = navigateTabSchema.parse(params)
        return { list: await browser.navigate(tab, url) }
      }
      case 'history': {
        const { tab, action } = tabHistorySchema.parse(params)
        return { list: await browser.history(tab, action) }
      }
      case 'stop': {
        const { tab } = stopTabSchema.parse(params)
        return { list: await browser.stop(tab) }
      }
    }
    return unknownMethod(method)
  }
  return {
    downloading: () => browser.downloading.value,
    followState: () => ({
      // A browser held while the page connects has brought no list yet, as before a feed's first answer.
      value: () => (browser.held.value === 'connecting' ? undefined : browser.listed.value),
      error: () => null,
      read: () => {},
      stop: () => {},
    }),
    call: async (method, params) => {
      try {
        return await call(method, params)
      } catch (error) {
        // A refusal with its reason, as the backend answers one with 409; one without, as it answers the
        // plugin's own failure.
        if (error instanceof BrowserTabsError) {
          throw error.reason === null
            ? new PluginCallError('plugin_failed', error.message, { type: 'status', status: SERVER_FAILED })
            : new PluginCallError(error.reason, error.message, { type: 'status', status: '409 Conflict' })
        }
        throw error
      }
    },
    streams: { browser: browser.stream },
  }
}

/** The skills a fetch of `origin` finds in the gallery: three, named after the repository, and `review`. */
function fetchedSkills(origin: string): Omit<SourceState['skills'][number], 'enabled'>[] {
  const name = origin.replace(/\/+$/, '').replace(/\.git$/, '').split('/').at(-1) ?? 'skills'
  return [
    { name: `${name}-review`, description: 'Review a change before it lands.', warnings: [], disableModelInvocation: false },
    { name: `${name}-release`, description: 'Cut a release: bump, tag and publish the changelog.', warnings: [], disableModelInvocation: false },
    { name: `${name}-notes`, description: 'Write release notes the user asks for.', warnings: [], disableModelInvocation: true },
    { name: 'review', description: 'Review a change, the way this repository reviews.', warnings: [], disableModelInvocation: false },
  ]
}

/** `state` as the backend's plugin sends it: each skill that is off names the source whose skill of its name is on. */
function withTakenNames(state: SkillsState): SkillsState {
  const on = new Map<string, SourceState>()
  for (const source of state.sources) {
    for (const skill of source.skills.filter((candidate) => candidate.enabled)) {
      on.set(skill.name, source)
    }
  }
  return {
    sources: state.sources.map((source) => ({
      ...source,
      skills: source.skills.map((skill) => {
        const taker = skill.enabled ? undefined : on.get(skill.name)
        return { ...skill, takenBy: taker?.origin, takenIn: taker?.workspace ?? undefined }
      }),
    })),
  }
}

/**
 * A repository in the one form the backend's plugin keys sources by: the
 * URL, `owner/repo` on GitHub, with its host in lowercase and without a
 * trailing slash or `.git`.
 */
function galleryRepository(origin: string): string {
  const text = origin.trim()
  const rest = (text.startsWith('https://') ? text : `https://github.com/${text}`).slice('https://'.length)
  const slash = rest.indexOf('/')
  const host = (slash < 0 ? rest : rest.slice(0, slash)).toLowerCase()
  const path = (slash < 0 ? '' : rest.slice(slash + 1)).replace(/\/+$/, '').replace(/\.git$/, '')
  return `https://${host}/${path}`
}

/** A place's name as the backend's plugin says it: Global, or its workspace's. */
function galleryPlace(workspace: string | null | undefined): string {
  return workspace ? workspaceName(galleryWorkspaces.list.find((each) => each.id === workspace) ?? { name: null }) : 'Global'
}

/** A commit id for the gallery's fetches and skill sources. */
export function commitOf(seed: string): string {
  let hash = 0x811c9dc5
  for (const character of seed) {
    hash = Math.imul(hash ^ character.charCodeAt(0), 0x01000193) >>> 0
  }
  return hash.toString(16).padStart(8, '0').repeat(5)
}

/** The shortest key the gallery's Exa accepts: a shorter one is refused, as Exa refuses a key copied in part. */
export const GALLERY_EXA_KEY_LENGTH = 16

/**
 * The web plugin over `state`:
 *
 * - Sign In answers Exa's sign-in page, which the page opens in a new tab, and
 *   the user is signed in as alex@example.com a moment later, as Exa's callback
 *   would.
 * - Sign Out forgets the sign-in.
 * - A key is checked with one search, a beat long: one shorter than {@link
 *   GALLERY_EXA_KEY_LENGTH} is refused, as Exa refuses a key copied in part.
 * - The key in use follows: the key over the sign-in over neither.
 * - Search and fetch switch as the backend would, from their defaults.
 */
export function webPlugin(state: WebState): GalleryPlugin {
  const inUse = (): void => {
    state.inUse = state.key ? 'key' : state.account ? 'account' : 'anonymous'
  }
  return {
    features: reactive({ ...WEB_FEATURES }),
    // A new object at each change, as the backend sends the state anew.
    state: () => ({
      account: state.account ? { ...state.account } : null,
      key: state.key ? { ...state.key } : null,
      inUse: state.inUse,
    }),
    call: async (method, params) => {
      switch (method) {
        case 'sign_in':
          signInSchema.parse(params)
          await beat(300)
          // The tab the page opened comes back signed in.
          void beat(1500).then(() => {
            state.account = { email: 'alex@example.com' }
            inUse()
          })
          return { url: 'https://auth.exa.ai/authorize?client_id=demi&state=gallery' }
        case 'sign_out':
          signOutSchema.parse(params)
          await beat(400)
          state.account = null
          inUse()
          return null
        case 'save_key': {
          const { key } = saveKeySchema.parse(params)
          await beat(900)
          if (key.trim().length < GALLERY_EXA_KEY_LENGTH) {
            throw new PluginCallError('key_refused', 'Exa refused this key')
          }
          state.key = { lastFour: key.trim().slice(-4), unchecked: null }
          inUse()
          return null
        }
        case 'remove_key':
          removeKeySchema.parse(params)
          await beat(300)
          state.key = null
          inUse()
          return null
        default:
          return unknownMethod(method)
      }
    },
  }
}

/** Opens Exa's sign-in as the product would: a toast says so. */
export async function openExaSignIn(url: string | Promise<string>): Promise<void> {
  await url
  productWould('Open Exa’s Sign-In Page in a New Tab')
}

/**
 * The skills plugin over `state`:
 *
 * - Adding a source fetches it after a beat, and updating fetches it again.
 * - A skill turns on unless another on has its name, as the backend's plugin
 *   refuses it.
 * - A fetch keeps each known skill on or off and starts a new one on, unless
 *   another skill on has its name or it is never offered to the agent.
 * - A switch is answered after a beat, as the backend writes the change first.
 * - Checking for updates finds none: the fixture's repositories never move.
 */
export function skillsPlugin(state: SkillsState): GalleryPlugin {
  /** Refuses `fetcher` for `origin` as the backend's plugin does. */
  const checkFetcher = (origin: string, fetcher: string | null | undefined): void => {
    if (!fetcher) {
      if (/^[^/:]+@|^ssh:\/\//.test(origin.trim())) {
        throw new PluginCallError('ssh_needs_device', 'An SSH address needs a device to fetch it, with its own keys')
      }
      return
    }
    const device = galleryDevices().find((demo) => demo.device.id === fetcher)?.device
    if (!device) {
      throw new PluginCallError('fetcher_not_found', 'The device that fetches the source is not paired')
    }
    if (device.state !== 'online') {
      throw new PluginCallError('fetcher_offline', `${device.name} is offline`)
    }
  }
  const source = (id: string): SourceState => {
    const found = state.sources.find((candidate) => candidate.id === id)
    if (!found) {
      throw new PluginCallError('source_not_found', `No skill source "${id}"`)
    }
    return found
  }
  async function fetch(id: string): Promise<void> {
    const fetching = source(id)
    fetching.fetching = true
    await beat(900)
    const fetched = state.sources.find((candidate) => candidate.id === id)
    if (!fetched) {
      return
    }
    const known = new Map(fetched.skills.map((skill) => [skill.name, skill.enabled]))
    const taken = new Set(
      state.sources
        .filter((other) => other.id !== id)
        .flatMap((other) => other.skills.filter((skill) => skill.enabled).map((skill) => skill.name)),
    )
    fetched.skills = fetchedSkills(fetched.origin).map((skill) => {
      const wanted = known.get(skill.name) ?? !skill.disableModelInvocation
      const enabled = wanted && !taken.has(skill.name)
      if (enabled) {
        taken.add(skill.name)
      }
      return { ...skill, enabled }
    })
    fetched.commit = commitOf(`${fetched.origin}${Date.now()}`)
    fetched.fetchedAt = new Date().toISOString()
    fetched.failure = null
    fetched.updateAvailable = false
    fetched.fetching = false
  }
  function taken(id: string, names: readonly string[]): void {
    for (const other of state.sources) {
      for (const skill of other.skills) {
        if (skill.enabled && names.includes(skill.name) && other.id !== id) {
          throw new PluginCallError('skill_name_taken', `A skill named “${skill.name}” from ${other.origin}, in ${galleryPlace(other.workspace)}, is on. Turn it off first.`)
        }
      }
    }
  }
  return {
    state: () => withTakenNames(state),
    call: async (method, params) => {
      switch (method) {
        case 'add_source': {
          const { origin, workspace, fetcher } = addSourceSchema.parse(params)
          if (!/^(https:\/\/\S+\/\S+|ssh:\/\/\S+\/\S+|[^\s/:@]+@[^\s/:]+:\S+|[\w.-]+\/[\w.-]+)$/.test(origin.trim())) {
            throw new PluginCallError('invalid_origin', `"${origin}" is neither owner/repo, an https URL nor an SSH address`)
          }
          const existing = state.sources.find((candidate) => galleryRepository(candidate.origin) === galleryRepository(origin))
          if (existing) {
            throw new PluginCallError('source_exists', `${origin} is added already, in ${galleryPlace(existing.workspace)}`)
          }
          checkFetcher(origin, fetcher)
          const id = commitOf(origin).slice(0, 12)
          state.sources.push({ id, origin: origin.trim(), workspace, fetcher, fetching: true, updateAvailable: false, skills: [], skipped: [] })
          void fetch(id)
          return { source: id }
        }
        case 'set_fetcher': {
          const { source: id, fetcher } = setFetcherSchema.parse(params)
          checkFetcher(source(id).origin, fetcher)
          source(id).fetcher = fetcher
          void fetch(id)
          return null
        }
        case 'update_source': {
          const { source: id } = sourceCallSchema.parse(params)
          checkFetcher(source(id).origin, source(id).fetcher)
          if (!source(id).fetching) {
            void fetch(id)
          }
          return null
        }
        case 'remove_source': {
          const { source: id } = sourceCallSchema.parse(params)
          source(id)
          state.sources = state.sources.filter((candidate) => candidate.id !== id)
          return null
        }
        case 'set_enabled': {
          const { source: id, skill: name, enabled } = setEnabledSchema.parse(params)
          await beat(400)
          const skill = source(id).skills.find((candidate) => candidate.name === name)
          if (!skill) {
            throw new PluginCallError('skill_not_found', `The source has no skill "${name}"`)
          }
          if (enabled) {
            taken(id, [name])
          }
          skill.enabled = enabled
          return null
        }
        case 'set_source_enabled': {
          const { source: id, enabled } = setSourceEnabledSchema.parse(params)
          await beat(400)
          const skills = source(id).skills
          if (enabled) {
            taken(id, skills.map((skill) => skill.name))
          }
          for (const skill of skills) {
            skill.enabled = enabled
          }
          return null
        }
      }
      return unknownMethod(method)
    },
  }
}
