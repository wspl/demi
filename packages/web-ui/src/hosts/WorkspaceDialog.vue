<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { FolderOpen, Plus, X } from '@lucide/vue'
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
 * from a name, a device and a directory. The folder browser is a page of the same
 * dialog, opened by Browse…. Every opening starts from a clean form.
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
  /** Offer the managed Cloud workspace beside the devices. */
  cloud?: boolean
  /** The directory the form starts on for a device. */
  defaultPath?: string
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

const name = ref('')
const path = ref('')
const deviceId = ref('')
const showCreate = ref(false)
const browsing = ref(false)

const device = computed(() => props.devices.find((entry) => entry.id === deviceId.value) ?? null)
const deviceLabel = computed(() => (deviceId.value === 'cloud' ? 'Cloud' : device.value?.name ?? 'Choose a device'))
const browserHosts = computed(() => props.devices.map((entry) => ({ id: entry.id, label: entry.name, online: entry.online })))
const browserSource = computed(() => props.sourceFor(deviceId.value))
const browserPlaces = computed(() => props.placesFor?.(deviceId.value) ?? [])
const canCreate = computed(() => !!name.value.trim() && (deviceId.value === 'cloud' || (!!device.value && path.value.startsWith('/'))))

watch(
  () => props.isOpen,
  (open) => {
    if (!open) return
    name.value = ''
    path.value = props.defaultPath ?? '/'
    deviceId.value = props.devices[0]?.id ?? (props.cloud ? 'cloud' : '')
    showCreate.value = props.mode === 'create'
    browsing.value = false
  },
  { immediate: true },
)

/** The browser's folder becomes the directory, and the name when none is typed. */
function pickDirectory(chosen: string) {
  path.value = chosen
  if (!name.value.trim()) name.value = chosen.split('/').filter(Boolean).at(-1) ?? ''
  browsing.value = false
}

function create() {
  if (!canCreate.value) return
  emit('create', { name: name.value.trim(), deviceId: deviceId.value, path: deviceId.value === 'cloud' ? '' : path.value.trim() })
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
          <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
            Project name
            <TextInput v-model="name" focused maxlength="64" placeholder="My next idea" />
          </label>
          <div class="flex flex-col gap-1.5 text-chrome text-fg-muted">
            Device
            <Dropdown :overlay-store="overlayStore" variant="default" trigger-label="Device">
              <template #trigger>{{ deviceLabel }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem
                    v-for="entry in devices"
                    :key="entry.id"
                    :label="entry.name"
                    :indicator="entry.online ? 'success' : 'muted'"
                    :indicator-label="entry.online ? 'Online' : 'Offline'"
                    choice
                    :is-selected="deviceId === entry.id"
                    @select="deviceId = entry.id; close()"
                  />
                  <MenuItem v-if="cloud" label="Cloud" choice :is-selected="deviceId === 'cloud'" @select="deviceId = 'cloud'; close()" />
                </Menu>
              </template>
            </Dropdown>
          </div>
          <label v-if="deviceId !== 'cloud'" class="flex flex-col gap-1.5 text-chrome text-fg-muted">
            Directory
            <span class="flex items-center gap-2">
              <TextInput v-model="path" placeholder="/path/to/project" class="min-w-0 flex-1" />
              <Button class="shrink-0" :disabled="!device?.online" @click="browsing = true">
                <FolderOpen :size="14" />
                Browse…
              </Button>
            </span>
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
