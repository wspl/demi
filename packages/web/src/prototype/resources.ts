import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { moveBefore } from '@demicodes/utils'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'
import type { SettingsProviderModel } from '@demicodes/web-ui/settings/types'
import { devices, projects } from './fixtures'
import { providers, settings, type PrototypeProvider } from './settings'

/** A provider can send when it is switched on and its last check passed. */
export function providerAvailable(provider: PrototypeProvider): boolean {
  return provider.enabled && provider.state === 'ready'
}

/** The composer's view of a model: what the settings hold, in the transport's shape. */
function modelInfo(provider: PrototypeProvider, model: SettingsProviderModel): ModelInfo {
  return {
    id: model.id,
    name: model.name || model.id,
    contextWindow: model.contextWindow,
    inputLimit: model.contextWindow,
    acceptedExtensions: model.extensions.map((extension) => extension.replace(/^\./, '')),
    reasoning: model.efforts.length
      ? {
        efforts: model.efforts,
        defaultEffort: model.efforts[Math.min(1, model.efforts.length - 1)] ?? null,
        canDisable: provider.kind !== 'subscription'
      }
      : null,
    serviceTiers: model.fastTier ? [
      {
        id: model.fastTier,
        label: 'Fast',
        fast: true
      }
    ] : null,
  }
}

export const useResources = defineStore('resources', {
  state: () => {
    const initialProjects = projects()
    return {
      projects: initialProjects,
      recentProjectIds: initialProjects.map((project) => project.id),
      devices: devices(),
      providers: providers(),
      selectedProviderId: 'claude-code' as string | null,
      providerDetailOpen: false,
      settings: settings(),
      username: 'Zan',
      settingsTab: 'general',
      settingsOpen: false,
      targetOpen: false,
      targetMode: 'switch' as 'switch' | 'create',
      /** The Add device pairing dialog, opened from anywhere a device could be connected. */
      pairingOpen: false,
      sidebarOpen: false,
    }
  },
  actions: {
    reorderProject(id: string, beforeId: string | null) {
      const item = this.projects.find((item) => item.id === id)
      const before = beforeId === null
        ? null
        : this.projects.find((item) => item.id === beforeId)
      if (!item || before === undefined)
        return
      this.projects = moveBefore(this.projects, item, before)
    },
    rememberProject(id: string) {
      this.recentProjectIds = [
        id,
        ...this.recentProjectIds.filter((item) => item !== id)
      ]
    },
  },
  getters: {
    signedIn: () => useSession().signedIn,
    /** What the composer lists: every provider, available when it can send. */
    providerInfos: (state): ProviderInfo[] =>
      state.providers.map(
        (provider) => ({
          id: provider.id,
          label: provider.name,
          isAvailable: providerAvailable(provider)
        })
      ),
    /** The models the composer offers per provider: the ones switched on in settings. */
    models: (state): Record<string, ModelInfo[]> =>
      Object.fromEntries(
        state.providers.map(
          (provider) => [
            provider.id,
            provider.models.filter((model) => model.enabled).map(
              (model) => modelInfo(provider, model)
            )
          ]
        ),
      ),
  },
})
