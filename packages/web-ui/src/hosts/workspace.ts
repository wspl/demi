import { computed, ref, watch, type ComputedRef } from 'vue'
import { useDirectoryExistence } from '../files/path-completion'
import type { FileBrowserSource } from '../files/types'
import type { PairingDevice } from '../devices/pairing'
import type { DeviceState } from '../devices/state'

/** What the working-environment dialog shows and asks for. Hosts map their own projects and devices onto these. */
export interface WorkspaceProject {
  id: string
  name: string
  /** Where it lives, as the row's value: a device name or `Cloud`. */
  host: string
  path: string
}

export interface WorkspaceDevice {
  id: string
  name: string
  state: DeviceState
}

/**
 * A new project as the form hands it back: on the Cloud only a name, the workspace being
 * managed; on a device a directory on it, the project taking the directory's name.
 */
export type WorkspaceDraft =
  | {
    kind: 'cloud';
    name: string
  }
  | {
    kind: 'device';
    deviceId: string;
    path: string
  }

/**
 * Where New Project starts: the kind the user last chose and the device last
 * chosen in its device menu, kept while the Cloud is chosen
 * (`product.md` § Conversations and projects).
 */
export interface WorkspaceHostChoice {
  kind: 'cloud' | 'device'
  deviceId?: string
}

/**
 * The choice a New Project form opens on: the remembered kind, the Cloud the
 * first time, and the remembered device while the user still has it, else no
 * device.
 */
export function openingChoice(
  remembered: WorkspaceHostChoice | undefined,
  devices: readonly WorkspaceDevice[],
): WorkspaceHostChoice {
  const kind = remembered?.kind ?? 'cloud'
  const deviceId = remembered?.deviceId
  if (deviceId && devices.some((device) => device.id === deviceId)) {
    return { kind, deviceId }
  }
  return { kind }
}

/**
 * Selects the device paired from beside a device menu in that menu. The claim
 * answers before the page's device list may carry the device, so the menu
 * selects it once it is among `devices`. Call it in a component's setup: the
 * wait ends with the component.
 */
export function usePairedSelection(
  devices: () => readonly WorkspaceDevice[],
  select: (id: string) => void,
): (device: PairingDevice) => void {
  const awaited = ref<string | null>(null)
  watch(
    () =>
      awaited.value !== null &&
      devices().some((device) => device.id === awaited.value),
    (listed) => {
      if (!listed || awaited.value === null) {
        return
      }
      const id = awaited.value
      awaited.value = null
      select(id)
    },
  )
  return (device) => {
    awaited.value = device.id
  }
}

/** What New Project's button does with a device's directory: take it as it is, or make it first. */
export type DeviceProjectAction = 'Add Project' | 'Create Project'

/**
 * New Project's button for a directory on a device (`product.md`
 * § Conversations and projects): Add Project when the directory exists,
 * Create Project when Demi will create it, as the listing the directory
 * field completes from tells (`useDirectoryExistence`). While the listing is
 * on its way, or after it failed, the button keeps what it last said, so it
 * does not flicker as the user types. It starts on Add Project: the form
 * opens on the device's home, which exists. An empty `path` lists nothing,
 * for a form that is closed or not on a device. Call it in a component's
 * setup: the listing it shows goes with the component.
 */
export function useDeviceProjectAction(options: {
  source: () => Pick<FileBrowserSource, 'home'> & Partial<Pick<FileBrowserSource, 'showListing'>>
  path: () => string
}): ComputedRef<DeviceProjectAction> {
  const exists = useDirectoryExistence({ source: options.source, base: () => undefined, text: options.path })
  const known = ref(true)
  watch(exists, (answer) => {
    if (answer !== null) {
      known.value = answer
    }
  }, { immediate: true })
  return computed(() => (known.value ? 'Add Project' : 'Create Project'))
}
