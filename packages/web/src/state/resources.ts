import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  workspaceAnswerSchema,
  type CreateWorkspace,
  type DeviceDto,
  type SidebarReorder,
} from '../api/generated/web-api'
import { useSession } from '../auth/session'
import { useProduct } from './product'
import { usePreferences } from './preferences'
import { modelInfo, providerView, wireApi } from './catalog'
import { ASIDE_SHARE, SIDEBAR_WIDTH } from '@demicodes/web-ui/sidebar/sidebar-width'
import { isSettingsSectionEnabled } from '@demicodes/web-ui/settings/sections'
import { emptyLocalState, readLocalState, writeLocalState } from './local'
import type { Device, Project } from './types'

/** Product data and page state. Components own interaction and presentation. */
/** A snapshot device as the file browser and the work panel take it. */
function productDevice(device: DeviceDto): Device {
  return {
    id: device.id,
    kind: device.kind,
    name: device.name,
    online: device.online,
    home: device.home,
    seen: device.lastSeenAt ?? undefined,
    platform:
      device.platform === 'darwin' || device.platform === 'macos'
        ? 'macos'
        : device.platform === 'win32' || device.platform === 'windows'
          ? 'windows'
          : 'linux',
  }
}

export const useResources = defineStore('resources', () => {
  const session = useSession()
  const product = useProduct()
  const preferences = usePreferences()
  const local = ref(emptyLocalState())
  let controller = new AbortController()
  const selectedProviderId = ref<string | null>(null)
  const providerDetailOpen = ref(false)
  const settingsTab = ref('general')
  const settingsOpen = ref(false)
  /** Opens the settings dialog, on `tab` when it names an enabled section. */
  function openSettings(tab?: string) {
    if (tab && isSettingsSectionEnabled(tab)) {
      settingsTab.value = tab
    }
    settingsOpen.value = true
  }
  const targetOpen = ref(false)
  const pairingOpen = ref(false)
  const sidebarOpen = ref(false)

  watch(
    () => session.user?.id,
    (id) => {
      controller.abort()
      controller = new AbortController()
      settingsOpen.value = false
      targetOpen.value = false
      pairingOpen.value = false
      sidebarOpen.value = false
      selectedProviderId.value = null
      providerDetailOpen.value = false
      local.value = id ? readLocalState(id) : emptyLocalState()
    },
    { immediate: true },
  )
  watch(
    local,
    (value) => {
      if (session.user) {
        writeLocalState(session.user.id, value)
      }
    },
    { deep: true },
  )

  const signedIn = computed(() => session.signedIn)
  const username = computed(
    () => product.snapshot?.user.nickname ?? session.user?.nickname ?? '',
  )
  const email = computed(
    () => product.snapshot?.user.email ?? session.user?.email ?? '',
  )
  const canConfigure = computed(
    () =>
      !!product.snapshot &&
      (product.snapshot.mode === 'isolated' ||
        product.snapshot.user.role === 'master'),
  )
  const appearance = computed(() => preferences.appearance)
  const keys = computed(() => preferences.keys)
  const devices = computed<Device[]>(() =>
    (product.snapshot?.devices ?? [])
      .filter((device) => device.kind === 'user')
      .map(productDevice),
  )
  /** Any device of the snapshot by id, the user's Cloud included; null when unknown. */
  function deviceById(id: string | null): Device | null {
    const device = product.snapshot?.devices.find((candidate) => candidate.id === id)
    return device ? productDevice(device) : null
  }
  const projects = computed<Project[]>(() =>
    (product.snapshot?.workspaces ?? []).map((workspace) => {
      const device = product.snapshot?.devices.find(
        (candidate) => candidate.id === workspace.deviceId,
      )
      const cloud = device?.kind === 'managed'
      return {
        id: workspace.id,
        name: workspace.name,
        deviceId: workspace.deviceId,
        host: cloud ? 'Cloud' : (device?.name ?? 'Unavailable device'),
        hostKind: cloud ? 'cloud' : 'device',
        path: workspace.path,
      }
    }),
  )
  const providers = computed(() =>
    (product.snapshot?.providers ?? []).map((provider) =>
      providerView(
        provider,
        product.catalog.find((catalog) => catalog.providerId === provider.id),
        local.value,
      ),
    ),
  )
  // A provider that needs a process runs it on the user's Cloud, which is
  // always there to be woken, so no provider depends on the conversation's target.
  const providerInfos = computed(() => {
    const catalog = product.catalog
    return (product.snapshot?.providers ?? []).map(provider => ({
      id: provider.id,
      label: provider.label,
      isAvailable: !local.value.hiddenProviders.includes(provider.id) &&
        catalog.find(item => item.providerId === provider.id)?.availability.type === 'available',
    }))
  })

  function modelsFor() {
    return Object.fromEntries(
      product.catalog.map((provider) => [
        provider.providerId,
        provider.models
          .filter(
            (model) =>
              !local.value.hiddenModels[provider.providerId]?.includes(model.id),
          )
          .map(modelInfo),
      ]),
    )
  }

  const models = computed(() => modelsFor())
  const vendors = computed(() =>
    (product.vendors?.vendors ?? []).map((vendor) => ({
      id: vendor.id,
      name: vendor.name,
      baseUrl: vendor.baseUrl,
      wireApi: wireApi(vendor.providerType, vendor.wireApi),
      logo: '',
    })),
  )
  const sidebarWidth = computed({
    get: () => local.value.sidebarWidth ?? SIDEBAR_WIDTH.default,
    set: (width: number) => {
      local.value.sidebarWidth = width
    },
  })
  const asideShare = computed({
    get: () => local.value.asideShare ?? ASIDE_SHARE.default,
    set: (share: number) => {
      local.value.asideShare = share
    },
  })
  const recentProjectIds = computed(() => local.value.recentProjects)

  function rememberProject(id: string): void {
    local.value.recentProjects = [
      id,
      ...local.value.recentProjects.filter((candidate) => candidate !== id),
    ].slice(0, 30)
  }

  function hideProvider(id: string, visible: boolean): void {
    local.value.hiddenProviders = visible
      ? local.value.hiddenProviders.filter((candidate) => candidate !== id)
      : [...new Set([...local.value.hiddenProviders, id])]
  }

  function hideModel(providerId: string, modelId: string, visible: boolean): void {
    const hidden = local.value.hiddenModels[providerId] ?? []
    local.value.hiddenModels[providerId] = visible
      ? hidden.filter((id) => id !== modelId)
      : [...new Set([...hidden, modelId])]
  }

  async function reorderProject(id: string, beforeId: string | null): Promise<void> {
    await apiRequest('/sidebar/reorder', {
      method: 'POST',
      signal: controller.signal,
      ...jsonBody({
        kind: 'workspace',
        id,
        beforeId,
      } satisfies SidebarReorder),
    })
    await product.revalidate()
  }

  async function createProject(draft: CreateWorkspace): Promise<string> {
    const current = controller
    const response = await apiRequest('/workspaces', {
      method: 'POST',
      signal: controller.signal,
      ...jsonBody(draft),
    })
    const { workspace } = await readResponse(response, workspaceAnswerSchema)
    await product.revalidate()
    current.signal.throwIfAborted()
    rememberProject(workspace.id)
    return workspace.id
  }

  async function removeProject(id: string): Promise<void> {
    await apiRequest(`/workspaces/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      signal: controller.signal,
    })
    await product.revalidate()
  }

  return {
    signedIn,
    username,
    email,
    appearance,
    keys,
    canConfigure,
    projects,
    devices,
    providers,
    vendors,
    providerInfos,
    models,
    modelsFor,
    local,
    selectedProviderId,
    providerDetailOpen,
    settingsTab,
    settingsOpen,
    openSettings,
    targetOpen,
    pairingOpen,
    sidebarOpen,
    sidebarWidth,
    asideShare,
    deviceById,
    recentProjectIds,
    rememberProject,
    hideProvider,
    hideModel,
    reorderProject,
    createProject,
    removeProject,
  }
})
