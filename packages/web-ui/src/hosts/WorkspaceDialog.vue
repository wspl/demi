<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { FolderOpen, Plus, X } from '@lucide/vue'
import { CLOUD_HOST_ID, hostIcon } from './icons'
import { ICON_PX } from '../ui/icon-metrics'
import { baseName } from '../files/paths'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import TextInput from '../ui/TextInput.vue'
import InlineError from '../ui/InlineError.vue'
import FileBrowser from '../files/FileBrowser.vue'
import type { FileBrowserPlaceGroup, FileBrowserSource } from '../files/types'
import type { WorkspaceDevice, WorkspaceDraft, WorkspaceProject } from './workspace'

/**
 * A conversation's working environment: pick one of the projects, or make a new one
 * from a device and a directory on it; the project is named after the directory. The
 * folder browser is a page of the same dialog, opened by Browse…. Every opening starts
 * from a clean form on the Cloud workspace, or the first device when there is none.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** Open on the project list, or straight on the new-project form. */
  mode: 'switch' | 'create'
  projects: WorkspaceProject[]
  currentProjectId: string | null
  /** No switching while a turn runs. */
  locked?: boolean
  devices: WorkspaceDevice[]
  /** Offer the managed Cloud workspace beside the devices; it is the default. */
  cloud?: boolean
  /** What went wrong with the last Create, shown under the form. */
  message?: string
  /** The browser's tree for a device. */
  sourceFor: (deviceId: string) => FileBrowserSource
  placesFor?: (deviceId: string) => FileBrowserPlaceGroup[]
}>()

const emit = defineEmits<{
  close: []
  select: [projectId: string | null]
  create: [draft: WorkspaceDraft]
}>()

const path = ref('')
const deviceId = ref('')
const showCreate = ref(false)
const browsing = ref(false)

const device = computed(() => props.devices.find((entry) => entry.id === deviceId.value) ?? null)
const cloudChosen = computed(() => deviceId.value === CLOUD_HOST_ID)
const deviceLabel = computed(() => (cloudChosen.value ? 'Cloud' : device.value?.name ?? 'Choose a device'))
const browserHosts = computed(() => [
  ...props.devices.map((entry) => ({ id: entry.id, label: entry.name, online: entry.online })),
  ...(props.cloud ? [{ id: CLOUD_HOST_ID, label: 'Cloud', online: true }] : []),
])
const browserSource = computed(() => props.sourceFor(deviceId.value))
const browserPlaces = computed(() => props.placesFor?.(deviceId.value) ?? [])
const online = computed(() => cloudChosen.value || !!device.value?.online)
const canCreate = computed(() => online.value && path.value.startsWith('/') && !!baseName(path.value))
/** What the project will be called: the directory's name. */
const projectName = computed(() => baseName(path.value.replace(/\/$/, '')))

watch(
  () => props.isOpen,
  (open) => {
    if (!open) return
    deviceId.value = props.cloud ? CLOUD_HOST_ID : props.devices[0]?.id ?? ''
    showCreate.value = props.mode === 'create'
    browsing.value = false
  },
  { immediate: true },
)

// The directory starts at the chosen host's home; picking another host starts over there.
watch(
  deviceId,
  (id) => {
    path.value = id ? props.sourceFor(id).home : ''
  },
  { immediate: true },
)

function pickDirectory(chosen: string) {
  path.value = chosen
  browsing.value = false
}

function create() {
  if (!canCreate.value) return
  emit('create', { deviceId: deviceId.value, path: path.value.replace(/\/$/, '') || '/' })
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :size="browsing ? 'lg' : 'md'"
    label="Working environment"
    hide-close
    @close="emit('close')"
  >
    <div v-if="browsing" class="flex h-[32rem] min-h-0 flex-col">
      <header class="flex h-11 shrink-0 select-none items-center justify-between gap-2 border-b border-line pl-4 pr-2">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Select folder</h3>
        <IconButton :icon="X" variant="ghost" aria-label="Close" @click="emit('close')" />
      </header>
      <FileBrowser
        mode="directory"
        :source="browserSource"
        :initial-path="path.startsWith('/') ? path.replace(/\/$/, '') || '/' : undefined"
        :places="browserPlaces"
        :hosts="browserHosts"
        :host-id="deviceId"
        @select="pickDirectory"
        @cancel="browsing = false"
        @update:host-id="deviceId = $event"
      />
    </div>
    <template v-else>
      <header class="flex select-none items-center justify-between border-b border-line px-4 py-3">
        <h2 class="text-[15px] font-medium text-fg-emphasis">{{ showCreate ? 'New project' : 'Working environment' }}</h2>
        <IconButton :icon="X" variant="ghost" aria-label="Close" @click="emit('close')" />
      </header>
      <div class="flex flex-col gap-4 p-4">
        <template v-if="!showCreate">
          <Menu class="w-full" iconless>
            <MenuItem label="No project" choice :is-selected="!currentProjectId" :disabled="locked" @select="emit('select', null)" />
            <MenuItem
              v-for="project in projects"
              :key="project.id"
              :label="project.name"
              :value="project.host"
              :title="project.path"
              choice
              :is-selected="currentProjectId === project.id"
              :disabled="locked"
              @select="emit('select', project.id)"
            />
          </Menu>
          <div>
            <Button :disabled="locked" @click="showCreate = true">
              <Plus :size="14" />
              New project
            </Button>
          </div>
        </template>
        <form v-else class="flex flex-col gap-4" @submit.prevent="create">
          <div class="flex flex-col gap-1.5 text-chrome text-fg-muted">
            Device
            <Dropdown :overlay-store="overlayStore" variant="default" trigger-label="Device">
              <template #trigger>
                <span class="flex items-center gap-2">
                  <component :is="hostIcon({ id: deviceId })" :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
                  {{ deviceLabel }}
                </span>
              </template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem v-if="cloud" :icon="hostIcon({ id: CLOUD_HOST_ID })" label="Cloud" choice :is-selected="cloudChosen" @select="deviceId = CLOUD_HOST_ID; close()" />
                  <MenuItem
                    v-for="entry in devices"
                    :key="entry.id"
                    :icon="hostIcon(entry)"
                    :label="entry.name"
                    :indicator="entry.online ? 'success' : 'muted'"
                    :indicator-label="entry.online ? 'Online' : 'Offline'"
                    :note="entry.online ? undefined : 'offline'"
                    choice
                    :is-selected="deviceId === entry.id"
                    @select="deviceId = entry.id; close()"
                  />
                </Menu>
              </template>
            </Dropdown>
          </div>
          <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
            Directory
            <span class="flex items-center gap-2">
              <TextInput v-model="path" placeholder="/path/to/project" class="min-w-0 flex-1" />
              <Button class="shrink-0" :disabled="!online" @click="browsing = true">
                <FolderOpen :size="14" />
                Browse…
              </Button>
            </span>
            <span class="text-[12px] leading-4 text-fg-subtle">{{ projectName ? `The project will be called ${projectName}.` : 'The project takes the folder\'s name.' }}</span>
          </label>
          <InlineError v-if="message" :message="message" />
          <div>
            <Button variant="primary" :disabled="!canCreate" @click="create">Create project</Button>
          </div>
        </form>
      </div>
    </template>
  </Dialog>
</template>
