<script setup lang="ts">
import { computed, ref } from 'vue'
import { Folder, GitBranch } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { Conversation, Project } from '../prototype/types'
import { useResources } from '../prototype/resources'
import { browserHosts, fileSourceFor, placesFor } from '../prototype/files'
import { useConversations } from '../conversation/store'
import HostMenu from './HostMenu.vue'

const props = defineProps<{ project?: Project; conversation: Conversation }>()
const resources = useResources()
const conversations = useConversations()
const directoryOpen = ref(false)
const recentDirectories = computed(() =>
  resources.recentProjectIds
    .flatMap((id) => resources.projects.find((item) => item.id === id) ?? [])
    .filter((item) => item.deviceId === props.project?.deviceId)
    .slice(0, 8),
)
const browsingDevice = ref<string | null>(null)
const browserPath = ref<string | undefined>()
const locked = computed(() => !!props.conversation.stream || props.conversation.archived)
const browsingHost = computed(() => resources.devices.find((item) => item.id === browsingDevice.value) ?? null)
const browserSource = computed(() => fileSourceFor(browsingHost.value))
const browserPlaces = computed(() => placesFor(browsingHost.value, resources.projects))
const browserHostList = computed(() => browserHosts(resources.devices, true))

function browse(deviceId = props.project?.deviceId ?? 'cloud', cwd?: string) {
  directoryOpen.value = false
  browserPath.value =
    cwd ??
    (deviceId === props.project?.deviceId
      ? props.project.path
      : resources.projects.find((project) => project.deviceId === deviceId)?.path)
  browsingDevice.value = deviceId
}
/** Another device in the browser's sidebar: it starts over at that device's home. */
function switchBrowserHost(deviceId: string) {
  browserPath.value = undefined
  browsingDevice.value = deviceId
}
function selectRecent(id: string) {
  if (locked.value) return
  const project = resources.projects.find((item) => item.id === id)
  if (!project || project.deviceId !== props.project?.deviceId) return
  if (
    project.hostKind !== 'cloud' &&
    !resources.devices.find((item) => item.id === project.deviceId)?.online
  )
    return
  conversations.move([props.conversation.id], id)
  resources.rememberProject(id)
  directoryOpen.value = false
}
function selectFolder(path: string) {
  if (locked.value || !browsingDevice.value) return
  const deviceId = browsingDevice.value
  const device = resources.devices.find((item) => item.id === deviceId)
  if (deviceId !== 'cloud' && !device?.online) return
  let project = resources.projects.find((item) => item.deviceId === deviceId && item.path === path)
  if (!project) {
    project = {
      id: crypto.randomUUID(),
      name: path.split('/').filter(Boolean).at(-1) ?? '/',
      deviceId,
      host: device?.name ?? 'Cloud',
      hostKind: deviceId === 'cloud' ? 'cloud' : 'device',
      path,
      branch: null,
    }
    resources.projects.push(project)
  }
  conversations.move([props.conversation.id], project.id)
  resources.rememberProject(project.id)
  browsingDevice.value = null
}
</script>

<template>
  <div class="flex min-w-0 max-w-full items-center gap-1">
    <HostMenu :conversation="conversation" :project="project" @switch-main="browse" />
    <template v-if="project">
      <Dropdown
        v-model:open="directoryOpen"
        :overlay-store="appOverlayStore"
        class="min-w-0 [&>div]:min-w-0"
      >
        <template #trigger>
          <Tooltip class="min-w-0" :content="project.path">
            <Button class="max-w-full" variant="ghost" aria-label="Switch directory">
              <Folder :size="ICON_PX.in28" />
              <span class="max-w-32 truncate">{{ project.name }}</span>
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
              :is-selected="item.id === project.id"
              :disabled="
                locked ||
                (item.hostKind !== 'cloud' &&
                  !resources.devices.find((device) => device.id === item.deviceId)?.online)
              "
              @select="selectRecent(item.id)"
            />
            <MenuDivider />
            <MenuItem :icon="Folder" label="Choose another directory…" @select="browse()" />
          </Menu>
        </template>
      </Dropdown>
      <span
        v-if="project.branch"
        class="flex min-w-0 select-none items-center gap-2 px-2 text-chrome text-fg-muted"
        aria-label="Current branch"
        :title="project.branch"
      >
        <GitBranch class="shrink-0" :size="ICON_PX.in28" />
        <span class="max-w-32 truncate">{{ project.branch }}</span>
      </span>
    </template>
    <!-- Stays mounted, open by state, so it can leave the way it arrived. -->
    <FileBrowserDialog
      :is-open="!!browsingDevice"
      :overlay-store="appOverlayStore"
      mode="directory"
      :source="browserSource"
      :initial-path="browserPath"
      :places="browserPlaces"
      :hosts="browserHostList"
      :host-id="browsingDevice ?? undefined"
      confirm-label="Use this folder"
      :confirm-disabled="locked"
      @select="selectFolder"
      @close="browsingDevice = null"
      @update:host-id="switchBrowserHost"
    />
  </div>
</template>
