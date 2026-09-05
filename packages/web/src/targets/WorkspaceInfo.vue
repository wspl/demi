<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Cloud, Folder, GitBranch, Monitor, Plus } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { Conversation, Device, Project } from '../prototype/types'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'
import FileBrowser from './FileBrowser.vue'
import { branchNameError } from './branches'

const props = defineProps<{ project: Project; device?: Device; conversation: Conversation }>()
const resources = useResources()
const conversations = useConversations()
const deviceOpen = ref(false)
const branchOpen = ref(false)
const query = ref('')
const branchSearch = ref<{ focus: () => void }>()
watch(branchOpen, async (open) => {
  if (!open) return
  query.value = ''
  await nextTick()
  branchSearch.value?.focus()
})
const browsingDevice = ref<string | null>(null)
const browserPath = ref('')
const locked = computed(() => !!props.conversation.stream || props.conversation.archived)
const matches = computed(() =>
  props.project.branches.filter((branch) =>
    branch.toLowerCase().includes(query.value.toLowerCase()),
  ),
)
const createError = computed(() => branchNameError(query.value))

function browse(deviceId = props.project.deviceId) {
  deviceOpen.value = false
  browserPath.value =
    deviceId === props.project.deviceId
      ? props.project.path
      : (resources.projects.find((project) => project.deviceId === deviceId)?.path ?? '/workspace')
  browsingDevice.value = deviceId
}
function switchDevice(id: string) {
  if (locked.value) return
  if (id === props.project.deviceId) {
    deviceOpen.value = false
    return
  }
  browse(id)
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
      path,
      branch: null,
      branches: [],
    }
    resources.projects.push(project)
  }
  conversations.move([props.conversation.id], project.id)
  browsingDevice.value = null
}
function selectBranch(branch: string) {
  if (locked.value || !props.project.branches.includes(branch)) return
  props.project.branch = branch
  branchOpen.value = false
}
function createBranch() {
  if (locked.value || createError.value || props.project.branches.includes(query.value)) return
  props.project.branches.push(query.value)
  selectBranch(query.value)
  query.value = ''
}
</script>

<template>
  <div class="flex min-w-0 max-w-full items-center gap-1">
    <Dropdown
      class="min-w-0 [&>div]:min-w-0"
      v-model:open="deviceOpen"
      :overlay-store="appOverlayStore"
    >
      <template #trigger>
        <Button class="max-w-full" variant="ghost" aria-label="Switch device" :disabled="locked">
          <Monitor :size="ICON_PX.in28" />
          <span class="max-w-28 truncate">{{ device?.name ?? project.host }}</span>
          <span
            v-if="device"
            class="size-1.5 rounded-full"
            :class="device.online ? 'bg-on-success' : 'bg-on-warning'"
          />
        </Button>
      </template>
      <template #content>
        <Menu class="w-60">
          <MenuItem
            v-for="item in resources.devices"
            :key="item.id"
            :icon="Monitor"
            :label="item.name"
            choice
            :is-selected="item.id === project.deviceId"
            :disabled="locked || !item.online"
            :disabled-reason="!item.online ? 'Device offline' : undefined"
            @select="switchDevice(item.id)"
          />
          <MenuItem
            :icon="Cloud"
            label="Cloud"
            choice
            :is-selected="project.deviceId === 'cloud'"
            :disabled="locked"
            @select="switchDevice('cloud')"
          />
          <p class="px-2 py-1 text-[11px] text-fg-subtle">
            Choose a workspace on the selected device.
          </p>
        </Menu>
      </template>
    </Dropdown>
    <Tooltip class="min-w-0" :content="project.path">
      <Button
        class="max-w-full"
        variant="ghost"
        aria-label="Browse workspace files"
        @click="browse()"
      >
        <Folder :size="ICON_PX.in28" />
        <span class="max-w-32 truncate">{{ project.name }}</span>
      </Button>
    </Tooltip>
    <Dropdown
      class="min-w-0 [&>div]:min-w-0"
      v-model:open="branchOpen"
      :overlay-store="appOverlayStore"
      placement="bottom-end"
    >
      <template #trigger>
        <Button class="max-w-full" variant="ghost" aria-label="Switch branch" :disabled="locked">
          <GitBranch :size="ICON_PX.in28" />
          <span class="max-w-32 truncate">{{ project.branch ?? 'No branch' }}</span>
        </Button>
      </template>
      <template #content>
        <Menu class="w-72" :autofocus="false">
          <div class="p-2">
            <TextInput
              ref="branchSearch"
              v-model="query"
              aria-label="Search or create branch"
              placeholder="Search or create branch…"
              @keydown.stop
            />
          </div>
          <div class="max-h-52 overflow-y-auto">
            <MenuItem
              v-for="branch in matches"
              :key="branch"
              :icon="GitBranch"
              :label="branch"
              choice
              :is-selected="branch === project.branch"
              :disabled="locked"
              @select="selectBranch(branch)"
            />
          </div>
          <p v-if="!matches.length" class="px-3 py-2 text-chrome text-fg-subtle">
            No matching branches.
          </p>
          <MenuDivider />
          <MenuItem
            :icon="Plus"
            :label="query ? `Create ${query}` : 'Type a name to create a branch'"
            :disabled="locked || !!createError || project.branches.includes(query)"
            @select="createBranch"
          />
          <p v-if="query && createError" class="px-3 py-2 text-[11px] text-on-warning">
            {{ createError }}
          </p>
          <p
            v-else-if="project.branches.includes(query)"
            class="px-3 py-2 text-[11px] text-fg-subtle"
          >
            This branch already exists. Select it above.
          </p>
          <p class="px-3 py-1 text-[11px] text-fg-subtle">
            Prototype branches · changes stay in memory.
          </p>
        </Menu>
      </template>
    </Dropdown>
    <FileBrowser
      v-if="browsingDevice"
      :device-id="browsingDevice"
      :initial-path="browserPath"
      :locked="locked"
      @close="browsingDevice = null"
      @select="selectFolder"
    />
  </div>
</template>
