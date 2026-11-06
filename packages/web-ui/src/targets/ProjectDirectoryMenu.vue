<script setup lang="ts">
import { t } from './locales/text'
import { computed, getCurrentInstance, ref } from 'vue'
import { operations, type Operation } from '../ui/operations'
import { Folder } from '@lucide/vue'
import WorktreeIcon from '@demicodes/web-ui/ui/WorktreeIcon.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import DirectoryChoice from '@demicodes/web-ui/files/DirectoryChoice.vue'
import type { OverlayStore } from '@demicodes/web-ui/overlay/overlayStore'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { COMPACT_LABEL_CLASS, useRoomLabel } from '../ui/label-room'
import type {
  FileBrowserDevice,
  FileBrowserSource,
  FileBrowserPlaceGroup,
} from '../files/types'

export interface DirectoryBrowserDevice extends FileBrowserDevice {
  source: FileBrowserSource
  places: FileBrowserPlaceGroup[]
}
const props = defineProps<{
  overlayStore: OverlayStore
  path: string | null
  projectName?: string | null
  selectedProjectId?: string
  deviceId: string | null
  locked: boolean
  browseEnabled: boolean
  /**
   * The projects on the device the conversation runs on, in the sidebar's
   * order: every directory a conversation runs in, but the home, is one.
   */
  projects: {
    id: string
    path: string
    disabled: boolean
  }[]
  devices: DirectoryBrowserDevice[]
  /**
   * The linked worktree of the project it works in, by git's name, which a
   * label after the directory shows; no place to choose, so it opens nothing.
   */
  worktree?: { name: string; path: string } | null
  selectFolder: (deviceId: string, path: string) => Promise<boolean>
}>()
const emit = defineEmits<{ selectProject: [id: string] }>()
const directoryOpen = ref(false)
const directoryName = computed(
  () => props.projectName ?? props.path?.split('/').filter(Boolean).at(-1) ?? '/',
)
// The directory gives up its name before the device does: its tooltip already says the whole path.
const directoryLabel = ref<HTMLElement | null>(null)
const directoryCompact = useRoomLabel(directoryLabel, () => directoryName.value, 1)
const browsingDeviceId = ref<string | null>(null)
const browserOpen = ref(false)
const browserPath = ref<string | undefined>()
/** Choosing a folder, as an operation (`operations.ts`): Use This Folder shows it while it works. */
const choosing: Operation = { action: 'choose', target: `directory:${getCurrentInstance()?.uid ?? 0}` }
// The home of the device the conversation runs on, which writes each row's location from `~`.
const home = computed(() => props.devices.find((device) => device.id === props.deviceId)?.source.home)
const browsingDevice = computed(() =>
  props.devices.find((device) => device.id === browsingDeviceId.value),
)
function browse(deviceId = props.deviceId, cwd?: string | null) {
  directoryOpen.value = false
  if (props.locked || !deviceId) {
    return
  }
  browserPath.value =
    cwd ?? (deviceId === props.deviceId ? (props.path ?? undefined) : undefined)
  browsingDeviceId.value = deviceId
  browserOpen.value = true
}
function switchBrowserDevice(deviceId: string) {
  browserPath.value = undefined
  browsingDeviceId.value = deviceId
}
function selectProject(id: string) {
  directoryOpen.value = false
  emit('selectProject', id)
}
async function selectFolder(path: string) {
  const deviceId = browsingDeviceId.value
  if (props.locked || !deviceId) {
    return
  }
  await operations.run(choosing, async () => {
    // Done, the browser closes if it still shows the device it was chosen on.
    if ((await props.selectFolder(deviceId, path)) && browserOpen.value && browsingDeviceId.value === deviceId) {
      browserOpen.value = false
    }
  })
}
defineExpose({ browse })
</script>

<template>
  <div class="flex min-w-0 max-w-full items-center gap-1">
    <slot />
    <template v-if="path">
      <Dropdown
        v-model:open="directoryOpen"
        :overlay-store="overlayStore"
        width="shrink"
      >
        <template #trigger>
          <Tooltip :content="path">
            <Button
              class="max-w-full"
              variant="ghost"
              :aria-label="t('directory.switch')"
            >
              <Folder :size="ICON_PX.in28" class="shrink-0" />
              <span
                ref="directoryLabel"
                class="max-w-32 truncate leading-5"
                :class="directoryCompact ? COMPACT_LABEL_CLASS : ''"
              >{{ directoryName }}</span>
            </Button>
          </Tooltip>
        </template>
        <template #content>
          <!-- The menu lays its rows on one grid and scrolls itself; nothing wraps the items. -->
          <Menu>
            <DirectoryChoice
              v-for="item in projects"
              :key="item.id"
              :path="item.path"
              :home="home"
              :selected="item.id === selectedProjectId"
              :disabled="locked || item.disabled"
              @select="selectProject(item.id)"
            />
            <MenuDivider v-if="projects.length" />
            <MenuItem
              :icon="Folder"
              :label="t('directory.chooseAnother')"
              :disabled="locked || !browseEnabled"
              @select="browse()"
            />
          </Menu>
        </template>
      </Dropdown>
      <Tooltip v-if="worktree" :content="worktree.path">
        <span
          class="flex h-hit min-w-0 cursor-default select-none items-center gap-1 px-1 text-chrome text-fg-muted phone:px-1.5"
          :aria-label="t('directory.worktree', { name: worktree.name })"
        >
          <WorktreeIcon :size="ICON_PX.in28" class="shrink-0" />
          <span class="max-w-32 truncate leading-5">{{ worktree.name }}</span>
        </span>
      </Tooltip>
    </template>
    <!-- Stays mounted, open by state, so it can leave the way it arrived. -->
    <FileBrowserDialog
      :is-open="browserOpen"
      :overlay-store="overlayStore"
      mode="directory"
      :source="browsingDevice.source"
      v-if="browsingDevice"
      :initial-path="browserPath"
      :places="browsingDevice.places"
      :devices="devices"
      :device-id="browsingDeviceId ?? undefined"
      :confirm-label="t('directory.useThis')"
      :confirm-disabled="locked"
      :confirm-operation="choosing"
      @select="selectFolder"
      @close="browserOpen = false"
      @update:device-id="switchBrowserDevice"
    />
  </div>
</template>
