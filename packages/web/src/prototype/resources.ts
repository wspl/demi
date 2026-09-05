import { defineStore } from 'pinia'
import { moveBefore } from '@demicodes/utils'
import { devices, projects, providers } from './fixtures'

export const useResources = defineStore('resources', {
  state: () => {
    const initialProjects = projects()
    return {
      projects: initialProjects,
      recentProjectIds: initialProjects.map((project) => project.id),
      devices: devices(),
      providers: providers(),
      username: 'Zan',
      signedIn: true,
      settingsTab: 'Account',
      settingsOpen: false,
      targetOpen: false,
      targetMode: 'switch' as 'switch' | 'create',
      sidebarOpen: false,
    }
  },
  actions: {
    reorderProject(id: string, beforeId: string | null) {
      const item = this.projects.find((item) => item.id === id)
      const before = beforeId === null ? null : this.projects.find((item) => item.id === beforeId)
      if (!item || before === undefined) return
      this.projects = moveBefore(this.projects, item, before)
    },
    rememberProject(id: string) {
      this.recentProjectIds = [id, ...this.recentProjectIds.filter((item) => item !== id)]
    },
  },
  getters: {
    models: (state) =>
      Object.fromEntries(state.providers.map((provider) => [provider.id, provider.models])),
  },
})
