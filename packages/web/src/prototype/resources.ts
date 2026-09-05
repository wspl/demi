import { defineStore } from 'pinia'
import { devices, projects, providers } from './fixtures'

export const useResources = defineStore('resources', {
  state: () => ({
    projects: projects(),
    devices: devices(),
    providers: providers(),
    username: 'Zan',
    signedIn: true,
    settingsTab: 'Account',
    settingsOpen: false,
    targetOpen: false,
    targetMode: 'switch' as 'switch' | 'create',
    sidebarOpen: false,
  }),
  getters: {
    models: (state) =>
      Object.fromEntries(state.providers.map((provider) => [provider.id, provider.models])),
  },
})
