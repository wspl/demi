<script setup lang="ts">
import { computed, ref } from 'vue'
import { Folder } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import type { OverlayStore } from '@demicodes/web-ui/overlay/overlayStore'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
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
const browsingDevice = ref<string | null>(null)
const browserOpen = ref(false)
const browserPath = ref<string | undefined>()
const pending = ref(false)
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
  if (props.locked || !deviceId || pending.value) {
    return
  }
  pending.value = true
  try {
    if (
      (await props.selectFolder(deviceId, path)) &&
      browsingDevice.value === deviceId
    ) {
      browserOpen.value = false
    }
  } finally {
    pending.value = false
  }
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
        class="min-w-0 [&>div]:min-w-0"
      >
        <template #trigger>
          <Tooltip class="min-w-0" :content="path">
            <Button
              class="max-w-full"
              variant="ghost"
              aria-label="Switch directory"
            >
              <Folder :size="ICON_PX.in28" />
              <span class="max-w-32 truncate">{{
                workspaceName ?? path.split('/').filter(Boolean).at(-1) ?? '/'
              }}</span>
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
              :label="item.path"
              choice
              :is-selected="item.id === selectedProjectId"
              :disabled="locked || item.disabled"
              @select="selectRecent(item.id)"
            />
            <MenuDivider />
            <MenuItem
              :icon="Folder"
              label="Choose another directory…"
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
      confirm-label="Use this folder"
      :confirm-disabled="locked"
      :confirm-pending="pending"
      @select="selectFolder"
      @close="browserOpen = false"
      @update:host-id="switchBrowserHost"
    />
  </div>
</template>
