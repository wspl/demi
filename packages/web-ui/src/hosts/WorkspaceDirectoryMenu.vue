<script setup lang="ts">
import { t } from './locales/text'
import { computed, getCurrentInstance, ref } from 'vue'
import { operations, type Operation } from '../ui/operations'
import { Folder } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import DirectoryLabel from '@demicodes/web-ui/files/DirectoryLabel.vue'
import { directoryParts } from '@demicodes/web-ui/files/paths'
import type { OverlayStore } from '@demicodes/web-ui/overlay/overlayStore'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { COMPACT_LABEL_CLASS, useRoomLabel } from '../ui/label-room'
import type {
  FileBrowserHost,
  FileBrowserSource,
  FileBrowserPlaceGroup,
} from '../files/types'

export interface DirectoryBrowserHost extends FileBrowserHost {
  source: FileBrowserSource
  places: FileBrowserPlaceGroup[]
}
const props = defineProps<{
  overlayStore: OverlayStore
  path: string | null
  workspaceName?: string | null
  selectedProjectId?: string
  deviceId: string | null
  locked: boolean
  browseEnabled: boolean
  recentDirectories: {
    id: string
    path: string
    disabled: boolean
  }[]
  hosts: DirectoryBrowserHost[]
  selectFolder: (deviceId: string, path: string) => Promise<boolean>
}>()
const emit = defineEmits<{ selectRecent: [id: string] }>()
const directoryOpen = ref(false)
const directoryName = computed(
  () => props.workspaceName ?? props.path?.split('/').filter(Boolean).at(-1) ?? '/',
)
// The directory gives up its name before the host does: its tooltip already says the whole path.
const directoryLabel = ref<HTMLElement | null>(null)
const directoryCompact = useRoomLabel(directoryLabel, () => directoryName.value, 1)
const browsingDevice = ref<string | null>(null)
const browserOpen = ref(false)
const browserPath = ref<string | undefined>()
/** Choosing a folder, as an operation (`operations.ts`): Use This Folder shows it while it works. */
const choosing: Operation = { action: 'choose', target: `directory:${getCurrentInstance()?.uid ?? 0}` }
// The home of the Host the conversation runs on, which writes each row's location from `~`.
const home = computed(() => props.hosts.find((host) => host.id === props.deviceId)?.source.home)
const browsingHost = computed(() =>
  props.hosts.find((host) => host.id === browsingDevice.value),
)
function browse(deviceId = props.deviceId, cwd?: string | null) {
  directoryOpen.value = false
  if (props.locked || !deviceId) {
    return
  }
  browserPath.value =
    cwd ?? (deviceId === props.deviceId ? (props.path ?? undefined) : undefined)
  browsingDevice.value = deviceId
  browserOpen.value = true
}
function switchBrowserHost(deviceId: string) {
  browserPath.value = undefined
  browsingDevice.value = deviceId
}
function selectRecent(id: string) {
  directoryOpen.value = false
  emit('selectRecent', id)
}
async function selectFolder(path: string) {
  const deviceId = browsingDevice.value
  if (props.locked || !deviceId) {
    return
  }
  await operations.run(choosing, async () => {
    // Done, the browser closes if it still shows the device it was chosen on.
    if ((await props.selectFolder(deviceId, path)) && browserOpen.value && browsingDevice.value === deviceId) {
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
                class="max-w-32 truncate"
                :class="directoryCompact ? COMPACT_LABEL_CLASS : ''"
              >{{ directoryName }}</span>
            </Button>
          </Tooltip>
        </template>
        <template #content>
          <!-- The menu lays its rows on one grid and scrolls itself; nothing wraps the items. -->
          <Menu>
            <MenuItem
              v-for="item in recentDirectories"
              :key="item.id"
              :icon="Folder"
              :label="directoryParts(item.path, home).name"
              :hint="item.path"
              choice
              :is-selected="item.id === selectedProjectId"
              :disabled="locked || item.disabled"
              @select="selectRecent(item.id)"
            >
              <DirectoryLabel :path="item.path" :home="home" />
            </MenuItem>
            <MenuDivider />
            <MenuItem
              :icon="Folder"
              :label="t('directory.chooseAnother')"
              :disabled="locked || !browseEnabled"
              @select="browse()"
            />
          </Menu>
        </template>
      </Dropdown>
    </template>
    <!-- Stays mounted, open by state, so it can leave the way it arrived. -->
    <FileBrowserDialog
      :is-open="browserOpen"
      :overlay-store="overlayStore"
      mode="directory"
      :source="browsingHost.source"
      v-if="browsingHost"
      :initial-path="browserPath"
      :places="browsingHost.places"
      :hosts="hosts"
      :host-id="browsingDevice ?? undefined"
      :confirm-label="t('directory.useThis')"
      :confirm-disabled="locked"
      :confirm-operation="choosing"
      @select="selectFolder"
      @close="browserOpen = false"
      @update:host-id="switchBrowserHost"
    />
  </div>
</template>
