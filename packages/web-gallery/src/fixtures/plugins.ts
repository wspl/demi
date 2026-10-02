import { BrowserTabsError, NEW_TAB_URL, type BrowserTabsApi } from '@demicodes/web-ui/browser/tabs'
import type { OpenLiveStream } from '@demicodes/web-ui/browser/session'
import { PluginCallError, type PluginHost } from '@demicodes/web-ui/plugins/client'
import { closeTabSchema, navigateTabSchema, openTabSchema, tabHistorySchema } from '@demicodes/plugin-browser'
import { exposeCallSchema, type ExposeState } from '@demicodes/plugin-expose'
import {
  addSourceSchema,
  setEnabledSchema,
  setSourceEnabledSchema,
  sourceCallSchema,
  type SkillsState,
  type SourceState,
} from '@demicodes/plugin-skills'

/**
 * The plugins as the gallery's specimens answer them (`plugins.md` § The
 * page): each over the specimen's own state, so every control of a plugin's
 * slot acts on it the way the backend's plugin would, after a beat where the
 * plugin would reach a Host or git.
 */
export interface GalleryPlugin {
  /** The state the plugin gives the user's pages, read reactively. */
  state?(): unknown
  call(method: string, params: object, conversation: string | null): Promise<unknown>
  /** Its user streams, by name. */
  streams?: Record<string, OpenLiveStream>
}

/** A host over `plugins`; a plugin it lacks refuses as the backend would. */
export function galleryPluginHost(plugins: Record<string, GalleryPlugin>): PluginHost {
  return {
    state: (plugin) => plugins[plugin]?.state?.(),
    call: async (plugin, method, params, conversation) => {
      const fixture = plugins[plugin]
      if (!fixture) {
        throw new PluginCallError('unknown_plugin', `No plugin "${plugin}"`)
      }
      return fixture.call(method, params, conversation)
    },
    stream: (name) => {
      for (const plugin of Object.values(plugins)) {
        const stream = plugin.streams?.[name]
        if (stream) {
          return stream
        }
      }
      throw new PluginCallError('unknown_stream', `No user stream "${name}"`)
    },
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

/** The conversation browser's plugin over the gallery's browser `api`. */
export function browserPlugin(api: BrowserTabsApi): GalleryPlugin {
  async function call(method: string, params: object): Promise<unknown> {
    switch (method) {
      case 'tabs':
        return api.list()
      case 'open':
        return { tab: await api.open(openTabSchema.parse(params).url ?? NEW_TAB_URL) }
      case 'close':
        await api.close(closeTabSchema.parse(params).tab)
        return null
      case 'navigate': {
        const { tab, url } = navigateTabSchema.parse(params)
        await api.navigate(tab, url)
        return null
      }
      case 'history': {
        const { tab, action } = tabHistorySchema.parse(params)
        await api.history(tab, action)
        return null
      }
    }
    return unknownMethod(method)
  }
  return {
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
    streams: { browser: api.stream },
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

/** The skills a fetch of `origin` finds in the gallery: three, named after the repository. */
function fetchedSkills(origin: string): SourceState['skills'] {
  const name = origin.replace(/\/+$/, '').replace(/\.git$/, '').split('/').at(-1) ?? 'skills'
  return [
    { name: `${name}-review`, description: 'Review a change before it lands.', warnings: [], enabled: false, disableModelInvocation: false },
    { name: `${name}-release`, description: 'Cut a release: bump, tag and publish the changelog.', warnings: [], enabled: false, disableModelInvocation: false },
    { name: `${name}-notes`, description: 'Write release notes the user asks for.', warnings: [], enabled: false, disableModelInvocation: true },
  ]
}

/** A commit id for the gallery's fetches. */
function commitOf(seed: string): string {
  let hash = 0x811c9dc5
  for (const character of seed) {
    hash = Math.imul(hash ^ character.charCodeAt(0), 0x01000193) >>> 0
  }
  return hash.toString(16).padStart(8, '0').repeat(5)
}

/**
 * The skills plugin over `state`: adding a source fetches it after a beat,
 * updating fetches it again, and a skill turns on unless another on has its
 * name, as the backend's plugin refuses it.
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
    const on = new Set(fetched.skills.filter((skill) => skill.enabled).map((skill) => skill.name))
    fetched.skills = fetchedSkills(fetched.origin).map((skill) => ({ ...skill, enabled: on.has(skill.name) }))
    fetched.commit = commitOf(`${fetched.origin}${Date.now()}`)
    fetched.fetchedAt = new Date().toISOString()
    fetched.failure = null
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
    state: () => state,
    call: async (method, params) => {
      switch (method) {
        case 'add_source': {
          const { origin } = addSourceSchema.parse(params)
          if (!/^(https:\/\/\S+\/\S+|[\w.-]+\/[\w.-]+)$/.test(origin.trim())) {
            throw new PluginCallError('invalid_origin', `"${origin}" is neither owner/repo nor an https URL`)
          }
          if (state.sources.some((existing) => existing.origin === origin.trim())) {
            throw new PluginCallError('source_exists', `${origin} is added already`)
          }
          const id = commitOf(origin).slice(0, 12)
          state.sources.push({ id, origin: origin.trim(), fetching: true, skills: [], skipped: [] })
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
