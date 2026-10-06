import { BrowserTabsError } from '@demicodes/plugin-browser/live/tabs'
import type { OpenUserStream } from '@demicodes/web-ui/plugins/streams'
import type { HostArtifact } from '@demicodes/web-ui/devices/installed'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import {
  PluginCallError,
  type ConversationFileService,
  type IntentService,
  type PageHost,
  type StateFeed,
} from '@demicodes/web-ui/plugins/page'
import { bindTabSchema, navigateTabSchema, syncTabsSchema, tabHistorySchema } from '@demicodes/plugin-browser'
import { exposeCallSchema, type ExposeState } from '@demicodes/plugin-expose'
import {
  addSourceSchema,
  checkUpdatesSchema,
  setEnabledSchema,
  setSourceEnabledSchema,
  sourceCallSchema,
  type SkillsState,
  type SourceState,
} from '@demicodes/plugin-skills'
import { productWould } from '../product-would'
import type { GalleryBrowser } from './live-browser'
import type { GalleryBrowserPlugin } from './panel'

/**
 * The plugins as the gallery's specimens answer them (`plugins.md` § The
 * page): each over the specimen's own state, so every control of a plugin's
 * page acts on it the way the backend's plugin would, after a beat where the
 * plugin would reach a Host or git.
 */
export interface GalleryPlugin {
  /** The plugin's user state, read reactively. */
  state?(): unknown
  /** Follows the plugin's state of a conversation, as the product follows it by revision. */
  followState?(conversation: string): StateFeed
  call(method: string, params: object, conversation: string | null): Promise<unknown>
  /** Its user streams, by name. */
  streams?: Record<string, OpenUserStream>
  /** What the specimen's Host holds of its packages, read reactively. */
  installed?(): readonly HostArtifact[]
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
}

/** A page host over `plugins` and the specimen's `shell`; a plugin it lacks refuses as the backend would. */
export function galleryPageHost(plugins: Record<string, GalleryPlugin>, shell: GalleryShell = {}): PageHost {
  const fixture = (plugin: string): GalleryPlugin => {
    const found = plugins[plugin]
    if (!found) {
      throw new PluginCallError('unknown_plugin', `No plugin "${plugin}"`)
    }
    return found
  }
  return {
    userState: (plugin) => plugins[plugin]?.state?.(),
    followState: (plugin, conversation) => {
      const follow = fixture(plugin).followState
      if (!follow) {
        throw new PluginCallError('unknown_plugin', `The plugin "${plugin}" has no conversation state`)
      }
      return follow(conversation)
    },
    call: async (plugin, method, params, conversation) => fixture(plugin).call(method, params, conversation),
    stream: (name) => {
      for (const plugin of Object.values(plugins)) {
        const stream = plugin.streams?.[name]
        if (stream) {
          return stream
        }
      }
      throw new PluginCallError('unknown_stream', `No user stream "${name}"`)
    },
    installed: (plugin) => plugins[plugin]?.installed?.() ?? [],
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
      add: (_conversation, kind) => productWould(`Open a ${kind} Tab in the Work Panel`),
      select: () => productWould('Select the Tab in the Work Panel'),
    },
    openSettings: (section) => productWould(`Open ${section} Settings`),
    overlays: appOverlayStore,
  }
}

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
        await panel.bind(bindTabSchema.parse(params).panelTab)
        return null
      case 'sync':
        syncTabsSchema.parse(params)
        await panel.sync()
        return null
      case 'navigate': {
        const { tab, url } = navigateTabSchema.parse(params)
        await browser.navigate(tab, url)
        return null
      }
      case 'history': {
        const { tab, action } = tabHistorySchema.parse(params)
        await browser.history(tab, action)
        return null
      }
    }
    return unknownMethod(method)
  }
  return {
    followState: () => ({
      value: () => browser.listed.value,
      error: () => null,
      read: () => {},
      stop: () => {},
    }),
    call: async (method, params) => {
      try {
        return await call(method, params)
      } catch (error) {
        if (error instanceof BrowserTabsError) {
          throw new PluginCallError(error.code ?? 'failed', error.message)
        }
        throw error
      }
    },
    streams: { browser: browser.stream },
    installed: () => browser.installed(),
  }
}

/** The expose plugin over `state`: renew moves an expiry to an hour from now, remove drops the expose. */
export function exposePlugin(state: ExposeState): GalleryPlugin {
  return {
    state: () => state,
    call: async (method, params) => {
      const { expose } = exposeCallSchema.parse(params)
      await beat(600)
      const index = state.exposes.findIndex((entry) => entry.id === expose)
      if (index < 0) {
        throw new PluginCallError('expose_not_found', `No expose ${expose}`)
      }
      if (method === 'renew') {
        state.exposes[index]!.expiresAt = new Date(Date.now() + 60 * 60_000).toISOString()
        return null
      }
      if (method === 'remove') {
        state.exposes.splice(index, 1)
        return null
      }
      return unknownMethod(method)
    },
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
  const on = new Map<string, string>()
  for (const source of state.sources) {
    for (const skill of source.skills.filter((candidate) => candidate.enabled)) {
      on.set(skill.name, source.origin)
    }
  }
  return {
    sources: state.sources.map((source) => ({
      ...source,
      skills: source.skills.map((skill) => ({ ...skill, takenBy: skill.enabled ? undefined : on.get(skill.name) })),
    })),
  }
}

/** A commit id for the gallery's fetches and skill sources. */
export function commitOf(seed: string): string {
  let hash = 0x811c9dc5
  for (const character of seed) {
    hash = Math.imul(hash ^ character.charCodeAt(0), 0x01000193) >>> 0
  }
  return hash.toString(16).padStart(8, '0').repeat(5)
}

/**
 * The skills plugin over `state`: adding a source fetches it after a beat,
 * updating fetches it again, and a skill turns on unless another on has its
 * name, as the backend's plugin refuses it. A fetch keeps each known skill
 * on or off and starts a new one on, unless another skill on has its name or
 * it is never offered to the agent. A switch is answered after a beat, as
 * the backend writes the change first. Checking for updates finds none: the
 * fixture's repositories never move.
 */
export function skillsPlugin(state: SkillsState): GalleryPlugin {
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
          throw new PluginCallError('skill_name_taken', `A skill named "${skill.name}" from ${other.origin} is on; turn it off first`)
        }
      }
    }
  }
  return {
    state: () => withTakenNames(state),
    call: async (method, params) => {
      switch (method) {
        case 'check_updates':
          checkUpdatesSchema.parse(params)
          return null
        case 'add_source': {
          const { origin } = addSourceSchema.parse(params)
          if (!/^(https:\/\/\S+\/\S+|[\w.-]+\/[\w.-]+)$/.test(origin.trim())) {
            throw new PluginCallError('invalid_origin', `"${origin}" is neither owner/repo nor an https URL`)
          }
          if (state.sources.some((existing) => existing.origin === origin.trim())) {
            throw new PluginCallError('source_exists', `${origin} is added already`)
          }
          const id = commitOf(origin).slice(0, 12)
          state.sources.push({ id, origin: origin.trim(), fetching: true, updateAvailable: false, skills: [], skipped: [] })
          void fetch(id)
          return { source: id }
        }
        case 'update_source': {
          const { source: id } = sourceCallSchema.parse(params)
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
