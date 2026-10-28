import { keptComputed } from '@demicodes/web-ui/infra/keep-same'
import { computed, ref, shallowRef, watch } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue, moveBefore } from '@demicodes/utils'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { RememberedChoices } from '@demicodes/web-ui/agent/starting-choices'
import { t } from './locales/text'
import {
  projectAnswerSchema,
  type ChangeProject,
  type ConversationTarget,
  type CreateProject,
  type DeviceDto,
  type SavedScopeSwitch,
  type SidebarReorder,
} from '../api/generated/web-api'
import type { SavedScope } from '@demicodes/protocol'
import { useSession } from '../auth/session'
import { known, useProduct } from './product'
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
import { deviceMarkOf, hostKindOf } from './hosts'
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
function productDevice(device: DeviceDto, devices: readonly DeviceDto[]): Device {
  const platform = FILE_BROWSER_PLATFORMS[device.platform]
  return {
    id: device.id,
    kind: device.kind,
    name: device.name,
    state: device.state,
    downloadPercent: device.downloadPercent,
    home: device.home,
    seen: device.lastSeenAt ?? undefined,
    platform,
    start: device.startCommand ? { command: device.startCommand, system: platform } : null,
    autostartUnavailable: device.autostartUnavailable,
    os: device.os,
    runnerVersion: device.runnerVersion,
    mark: deviceMarkOf(device, devices),
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
  /** The writes of what each project's next conversation starts with, one at a time per project. */
  const choices = new SerialQueue()
  const preferences = usePreferences()
  const local = ref(emptyLocalState())
  let controller = new AbortController()
  const selectedProviderId = ref<string | null>(null)
  const providerDetailOpen = ref(false)
  /** The sidebar's entries of the plugin pages the user has on, each opening its settings section. */
  const sectionEntries = keptComputed(() =>
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
    () => product.snapshot?.user?.nickname ?? session.user?.nickname ?? '',
  )
  const email = computed(
    () => product.snapshot?.user?.email ?? session.user?.email ?? '',
  )
  const canConfigure = computed(
    () =>
      product.snapshot?.mode === 'isolated' || product.snapshot?.user?.role === 'master',
  )
  const appearance = computed(() => preferences.appearance)
  const keys = keptComputed(() => preferences.keys)
  const devices = keptComputed<Device[]>(() =>
    (product.snapshot?.devices ?? [])
      .filter((device) => device.kind === 'user')
      .map((device) => productDevice(device, product.snapshot?.devices ?? [])),
  )
  /** Any device of the snapshot by id, the user's Cloud included; null when unknown. */
  function deviceById(id: string | null): Device | null {
    const device = product.snapshot?.devices?.find((candidate) => candidate.id === id)
    return device && product.snapshot?.devices ? productDevice(device, product.snapshot.devices) : null
  }
  /** A device as a row's mark shows it: by name and state, or as one the account no longer lists. */
  function sidebarDevice(deviceId: string): SidebarDevice {
    const devices = product.snapshot?.devices ?? []
    const device = devices.find((candidate) => candidate.id === deviceId)
    return device
      ? { kind: 'paired', name: device.name, state: device.state, downloadPercent: device.downloadPercent, mark: deviceMarkOf(device, devices) }
      : { kind: 'removed' }
  }
  /**
   * The device a conversation's sidebar row shows: for one outside a project on a paired device, its
   * name and state, or that it was removed; none on the Cloud, in a project, or before the first
   * snapshot names the devices.
   */
  function conversationDevice(target: ConversationTarget): SidebarDevice | undefined {
    return target.kind === 'device' && product.snapshot ? sidebarDevice(target.deviceId) : undefined
  }
  // A project that says what it said stays the same object, so its header is
  // not drawn again at each change of the state (`keepSame`).
  const projects = keptComputed((): Project[] =>
    (product.snapshot?.projects ?? []).map((project): Project => {
      const device = product.snapshot?.devices?.find(
        (candidate) => candidate.id === project.deviceId,
      )
      const checkout = {
        id: project.id,
        workspaceId: project.workspaceId,
        name: project.name,
        deviceId: project.deviceId,
        path: project.path,
        savedScope: project.savedScope,
        savedCount: project.savedCount,
      }
      if (hostKindOf(device) === 'cloud') {
        return { ...checkout, hostKind: 'cloud' }
      }
      return { ...checkout, hostKind: 'device', device: sidebarDevice(project.deviceId) }
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
  const providerInfos = keptComputed(() => {
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
  const models = keptComputed(() =>
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
      // A vendor of media alone speaks its own API, which no protocol names.
      wireApi: vendor.providerType === 'media' ? null : wireApi(vendor.providerType, vendor.wireApi),
      logo: vendorLogo(vendor.id),
    })),
  )
  const sidebarWidth = computed({
    get: () => local.value.sidebarWidth ?? SIDEBAR_WIDTH.default,
    set: (width: number) => {
      local.value.sidebarWidth = width
    },
  })
  const sidebarCollapsed = computed({
    get: () => local.value.sidebarCollapsed ?? false,
    set: (collapsed: boolean) => {
      local.value.sidebarCollapsed = collapsed
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
  /**
   * Moves the project before `beforeId`, or to the end, at once; the write
   * follows the earlier moves, and a failure, thrown, puts it back.
   */
  function reorderProject(id: string, beforeId: string | null): Promise<void> {
    const change = product.change('projects', (state) => {
      const projects = known(state.projects, 'projects')
      const moved = projects.find((project) => project.id === id)
      const before = projects.find((project) => project.id === beforeId) ?? null
      return moved && (beforeId === null || before)
        ? { ...state, projects: moveBefore(projects, moved, before) }
        : state
    })
    const { signal } = controller
    return moves
      .run('project_order', async () => {
        change.send()
        await apiRequest('/sidebar/reorder', {
          method: 'POST',
          signal,
          ...jsonBody({
            kind: 'project',
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

  /**
   * Makes the project `draft` names and answers its id; a directory that is
   * already a project answers that project, which the caller opens instead
   * of making a second.
   */
  async function createProject(draft: CreateProject): Promise<string> {
    const current = controller
    let id: string
    try {
      const response = await apiRequest('/projects', {
        method: 'POST',
        signal: controller.signal,
        ...jsonBody(draft),
      })
      id = (await readResponse(response, projectAnswerSchema)).project.id
    } catch (error) {
      if (!(error instanceof ApiError && error.code === 'project_exists' && error.projectId)) {
        throw error
      }
      id = error.projectId
    }
    // The caller opens the project, which the channel brings moments after
    // it was made.
    await product.until((state) => state.projects?.some((item) => item.id === id) ?? false, current.signal)
    return id
  }

  /**
   * Saves `choice`, made in a conversation of the project `id`, as what the
   * project's next conversation starts with: it shows at once, the write
   * follows the project's earlier ones, and the channel brings it to the user's
   * other pages.
   */
  function rememberInProject(id: string, choice: RememberedChoices): void {
    const change = product.change('projects', (state) => ({
      ...state,
      projects: known(state.projects, 'projects').map((project) => project.id === id ? { ...project, ...choice } : project),
    }))
    const { signal } = controller
    void choices
      .run(id, async () => {
        change.send()
        const response = await apiRequest(`/projects/${encodeURIComponent(id)}`, {
          method: 'PATCH',
          signal,
          ...jsonBody(choice satisfies ChangeProject),
        })
        const { project } = await readResponse(response, projectAnswerSchema)
        change.land((read) => ({
          type: 'projects',
          projects: known(read.projects, 'projects').map((candidate) => candidate.id === project.id ? project : candidate),
        }))
      })
      .catch((error) => {
        change.drop()
        if (!signal.aborted) {
          reportError(t('failures.updateSettings'), error, { site: 'failures.updateSettings', userVisible: true })
        }
      })
  }

  /**
   * Switches how the project `id`'s conversations keep their saved messages; to per conversation,
   * `to` names the conversation that takes the project's list. Done once the channel brings the
   * project's new way, so the menu's check never shows the old one after the switch.
   */
  async function switchSavedScope(id: string, scope: SavedScope, to: string | null): Promise<void> {
    const current = controller
    await apiRequest(`/projects/${encodeURIComponent(id)}/saved-scope`, {
      method: 'PUT',
      signal: current.signal,
      ...jsonBody({ scope, ...(to ? { to } : {}) } satisfies SavedScopeSwitch),
    })
    await product.until(
      (state) => state.projects?.some((item) => item.id === id && item.savedScope === scope) ?? true,
      current.signal,
    )
  }

  async function removeProject(id: string): Promise<void> {
    await apiRequest(`/projects/${encodeURIComponent(id)}`, {
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
    sidebarCollapsed,
    asideShare,
    dockWindowShare,
    passkeyOfferAnswered,
    deviceById,
    conversationDevice,
    reorderProject,
    createProject,
    removeProject,
    switchSavedScope,
    rememberInProject,
  }
})
