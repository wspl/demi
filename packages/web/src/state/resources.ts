import { computed, ref, shallowRef, watch } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue, moveBefore } from '@demicodes/utils'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  workspaceAnswerSchema,
  type ConversationTarget,
  type CreateWorkspace,
  type DeviceDto,
  type SidebarReorder,
} from '../api/generated/web-api'
import { useSession } from '../auth/session'
import { useProduct } from './product'
import { usePreferences } from './preferences'
import { modelInfo, providerView, setupOf, wireApi } from './catalog'
import { DOCK_WINDOW_SHARE } from '@demicodes/web-ui/agent/dock-window'
import { ASIDE_SHARE, SIDEBAR_WIDTH } from '@demicodes/web-ui/sidebar/sidebar-width'
import { sidebarEntries } from '@demicodes/web-ui/plugins/page'
import type { PairingDevice } from '@demicodes/web-ui/devices/pairing'
import { vendorLogo } from '@demicodes/web-ui/settings/vendor-logos'
import type { SidebarDevice } from '@demicodes/web-ui/sidebar/types'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { hostKindOf } from './hosts'
import { emptyLocalState, readLocalState, writeLocalState } from './local'
import type { Device, Project } from './types'

/** Product data and page state. Components own interaction and presentation. */
/** The file browser's name of each platform a runner reports. */
const FILE_BROWSER_PLATFORMS: Record<DeviceDto['platform'], Device['platform']> = {
  darwin: 'macos',
  win32: 'windows',
  linux: 'linux',
}

/** A snapshot device as the file browser and the work panel take it. */
function productDevice(device: DeviceDto): Device {
  const platform = FILE_BROWSER_PLATFORMS[device.platform]
  return {
    id: device.id,
    kind: device.kind,
    name: device.name,
    state: device.state,
    home: device.home,
    seen: device.lastSeenAt ?? undefined,
    platform,
    start: device.startCommand ? { command: device.startCommand, system: platform } : null,
    os: device.os,
    runnerVersion: device.runnerVersion,
  }
}

export const useResources = defineStore('resources', () => {
  const session = useSession()
  const product = useProduct()
  /**
   * The project moves on their way, which go one at a time in order: each
   * changes the one order of the projects.
   */
  const moves = new SerialQueue()
  const preferences = usePreferences()
  const local = ref(emptyLocalState())
  let controller = new AbortController()
  const selectedProviderId = ref<string | null>(null)
  const providerDetailOpen = ref(false)
  /** The sidebar's entries of the plugin pages the user has on, each opening its settings section. */
  const sectionEntries = computed(() =>
    sidebarEntries(PLUGIN_PAGES, (plugin) => pluginEnabled(product.snapshot, plugin)),
  )
  const targetOpen = ref(false)
  /**
   * The open pairing dialog and whom it tells of the device it pairs: the
   * device menu it was started beside, if any; null while it is closed.
   */
  const pairingRequest = shallowRef<{
    onPaired?: (device: PairingDevice) => void
  } | null>(null)
  function openPairing(onPaired?: (device: PairingDevice) => void) {
    pairingRequest.value = { onPaired }
  }
  const sidebarOpen = ref(false)

  watch(
    () => session.user?.id,
    (id) => {
      controller.abort()
      controller = new AbortController()
      targetOpen.value = false
      pairingRequest.value = null
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
  const userId = computed(() => session.user?.id ?? '')
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
  /** A device as a row's mark shows it: by name and state, or as one the account no longer lists. */
  function sidebarDevice(deviceId: string): SidebarDevice {
    const device = product.snapshot?.devices.find((candidate) => candidate.id === deviceId)
    return device ? { kind: 'paired', name: device.name, state: device.state } : { kind: 'removed' }
  }
  /**
   * The device a conversation's sidebar row shows (`product.md` § Conversations and projects):
   * for one outside a project on a paired device, its name and state, or that it was removed;
   * none on the Cloud, in a project, or before the first snapshot names the devices.
   */
  function conversationDevice(target: ConversationTarget): SidebarDevice | undefined {
    return target.kind === 'device' && product.snapshot ? sidebarDevice(target.deviceId) : undefined
  }
  const projects = computed<Project[]>(() =>
    (product.snapshot?.workspaces ?? []).map((workspace): Project => {
      const device = product.snapshot?.devices.find(
        (candidate) => candidate.id === workspace.deviceId,
      )
      const checkout = {
        id: workspace.id,
        name: workspace.name,
        deviceId: workspace.deviceId,
        path: workspace.path,
      }
      if (hostKindOf(device) === 'cloud') {
        return { ...checkout, hostKind: 'cloud' }
      }
      return { ...checkout, hostKind: 'device', device: sidebarDevice(workspace.deviceId) }
    }),
  )
  const hidden = { provider: preferences.hidesProvider, model: preferences.hidesModel }
  const providers = computed(() =>
    (product.snapshot?.providers ?? []).map((provider) =>
      providerView(
        provider,
        product.catalog.find((catalog) => catalog.providerId === provider.id),
        hidden,
      ),
    ),
  )
  // A provider that needs a process runs it on the user's Cloud, which is
  // always there to be woken, so no provider depends on the conversation's target.
  const providerInfos = computed(() => {
    const catalog = product.catalog
    return (product.snapshot?.providers ?? []).map(provider => {
      const setup = setupOf(catalog.find(item => item.providerId === provider.id))
      return {
        id: provider.id,
        label: provider.label,
        isAvailable: !preferences.hidesProvider(provider.id) &&
          catalog.find(item => item.providerId === provider.id)?.availability.type === 'available',
        // An incomplete entry's models show in the model menu dimmed, with the backend's reason.
        ...(setup && !preferences.hidesProvider(provider.id) ? { setupReason: setup.message } : {}),
      }
    })
  })

  /**
   * Every model each entry's catalog lists, with the context limit the user
   * stored on it, the ones this browser hides included: they name a
   * profile's model and tell whether it is gone.
   */
  const catalogModels = computed(() =>
    Object.fromEntries(
      product.catalog.map((provider) => [
        provider.providerId,
        provider.models.map((model) =>
          modelInfo(model, preferences.contextLimit(provider.providerId, model.id)),
        ),
      ]),
    ),
  )
  /**
   * The models the model menus offer, the composer's and a subagent
   * profile's alike: the catalog's, less the ones this browser hides.
   */
  const models = computed(() =>
    Object.fromEntries(
      Object.entries(catalogModels.value).map(([providerId, list]) => [
        providerId,
        list.filter((model) => !preferences.hidesModel(providerId, model.id)),
      ]),
    ),
  )
  const vendors = computed(() =>
    (product.vendors?.vendors ?? []).map((vendor) => ({
      id: vendor.id,
      name: vendor.name,
      baseUrl: vendor.baseUrl,
      wireApi: wireApi(vendor.providerType, vendor.wireApi),
      logo: vendorLogo(vendor.id),
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
  const dockWindowShare = computed({
    get: () => local.value.dockWindowShare ?? DOCK_WINDOW_SHARE.default,
    set: (share: number) => {
      local.value.dockWindowShare = share
    },
  })
  const passkeyOfferAnswered = computed({
    get: () => local.value.passkeyOfferAnswered ?? false,
    set: (answered: boolean) => {
      local.value.passkeyOfferAnswered = answered
    },
  })
  const recentProjectIds = computed(() => local.value.recentProjects)

  function rememberProject(id: string): void {
    local.value.recentProjects = [
      id,
      ...local.value.recentProjects.filter((candidate) => candidate !== id),
    ].slice(0, 30)
  }

  /**
   * Moves the project before `beforeId`, or to the end, at once; the write
   * follows the earlier moves, and a failure, thrown, puts it back.
   */
  function reorderProject(id: string, beforeId: string | null): Promise<void> {
    const change = product.change('workspaces', (state) => {
      const moved = state.workspaces.find((workspace) => workspace.id === id)
      const before = state.workspaces.find((workspace) => workspace.id === beforeId) ?? null
      return moved && (beforeId === null || before)
        ? { ...state, workspaces: moveBefore(state.workspaces, moved, before) }
        : state
    })
    const { signal } = controller
    return moves
      .run('workspace_order', async () => {
        change.send()
        await apiRequest('/sidebar/reorder', {
          method: 'POST',
          signal,
          ...jsonBody({
            kind: 'workspace',
            id,
            beforeId,
          } satisfies SidebarReorder),
        })
        change.land()
      })
      .catch((error) => {
        change.drop()
        throw error
      })
  }

  async function createProject(draft: CreateWorkspace): Promise<string> {
    const current = controller
    const response = await apiRequest('/workspaces', {
      method: 'POST',
      signal: controller.signal,
      ...jsonBody(draft),
    })
    const { workspace } = await readResponse(response, workspaceAnswerSchema)
    // The caller opens the project, which the channel brings moments after
    // it was made.
    await product.until((state) => state.workspaces.some((item) => item.id === workspace.id), current.signal)
    rememberProject(workspace.id)
    return workspace.id
  }

  async function removeProject(id: string): Promise<void> {
    await apiRequest(`/workspaces/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      signal: controller.signal,
    })
  }

  return {
    signedIn,
    userId,
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
    catalogModels,
    models,
    local,
    selectedProviderId,
    providerDetailOpen,
    sectionEntries,
    targetOpen,
    pairingRequest,
    openPairing,
    sidebarOpen,
    sidebarWidth,
    asideShare,
    dockWindowShare,
    passkeyOfferAnswered,
    deviceById,
    conversationDevice,
    recentProjectIds,
    rememberProject,
    reorderProject,
    createProject,
    removeProject,
  }
})
