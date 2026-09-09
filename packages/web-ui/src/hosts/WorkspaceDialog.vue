<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ChevronDown, Cloud, FolderOpen, Monitor, Plus, X } from '@lucide/vue'
import { CLOUD_HOST_ID, hostIcon } from './icons'
import { ICON_PX } from '../ui/icon-metrics'
import { baseName } from '../files/paths'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import ChoiceCards from '../ui/ChoiceCards.vue'
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
 * A conversation's working environment: pick one of the projects, or make a new one.
 * The form branches on where it lives: on the Cloud the workspace is managed and only
 * needs a name; on a device it is a directory there, and takes the directory's name.
 * The folder browser is a page of the same dialog, opened by Browse…. Every opening
 * starts from a clean form on Device, on the first device that is online.
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
  /** Offer the managed Cloud workspace; it is the default. */
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
  /** The Add device button beside the device menu: the host starts pairing. */
  connectDevice: []
}>()

type Kind = 'cloud' | 'device'
const kindOptions = [
  {
    value: 'device',
    label: 'Device',
    description: 'A directory on one of your devices.',
    icon: Monitor
  },
  {
    value: 'cloud',
    label: 'Cloud',
    description: 'A managed workspace, ready at once.',
    icon: Cloud
  },
] as const satisfies readonly {
  value: Kind;
  label: string;
  description: string;
  icon: typeof Cloud
}[]

const kind = ref<Kind>('device')
const name = ref('')
const path = ref('')
const deviceId = ref('')
const showCreate = ref(false)
const browsing = ref(false)

const device = computed(
  () => props.devices.find((entry) => entry.id === deviceId.value) ?? null
)
const deviceLabel = computed(() => device.value?.name ?? 'Choose a device')
const browserHosts = computed(
  () => props.devices.map(
    (entry) => ({
      id: entry.id,
      label: entry.name,
      online: entry.online
    })
  )
)
const browserSource = computed(() => props.sourceFor(deviceId.value))
const browserPlaces = computed(() => props.placesFor?.(deviceId.value) ?? [])
const online = computed(() => !!device.value?.online)
/** What a device project will be called: the directory's name. */
const projectName = computed(() => baseName(path.value.replace(/\/$/, '')))
const canCreate = computed(() =>
  kind.value === 'cloud'
    ? !!name.value.trim()
    : online.value && path.value.startsWith('/') && !!projectName.value,
)

watch(
  () => props.isOpen,
  (open) => {
    if (!open)
      return
    kind.value = 'device'
    name.value = ''
    deviceId.value = props.devices.find((entry) => entry.online)?.id ?? props.devices[0]?.id ?? ''
    showCreate.value = props.mode === 'create'
    browsing.value = false
  },
  { immediate: true },
)

// The directory starts at the chosen device's home; picking another device starts over there.
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
  if (!canCreate.value)
    return
  if (kind.value === 'cloud')
    emit('create', {
    kind: 'cloud',
    name: name.value.trim()
  })
  else emit(
    'create',
    {
      kind: 'device',
      deviceId: deviceId.value,
      path: path.value.replace(/\/$/, '') || '/'
    }
  )
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
      <header
        class="flex h-11 shrink-0 select-none items-center justify-between gap-2 border-b border-line pl-4 pr-2"
      >
        <h3 class="text-[15px] font-medium text-fg-emphasis">Select folder</h3>
        <IconButton
          :icon="X"
          variant="ghost"
          aria-label="Close"
          @click="emit('close')"
        />
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
      <header
        class="flex select-none items-center justify-between border-b border-line px-4 py-3"
      >
        <h2 class="text-[15px] font-medium text-fg-emphasis">{{ showCreate ? 'New project' : 'Working environment' }}</h2>
        <IconButton
          :icon="X"
          variant="ghost"
          aria-label="Close"
          @click="emit('close')"
        />
      </header>
      <div class="flex flex-col gap-4 p-4">
        <template v-if="!showCreate">
          <Menu class="w-full" iconless>
            <MenuItem
              label="No project"
              choice
              :is-selected="!currentProjectId"
              :disabled="locked"
              @select="emit('select', null)"
            />
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
        <form
          v-else
          class="flex flex-col gap-4"
          @submit.prevent="create"
        >
          <ChoiceCards
            v-if="cloud"
            v-model="kind"
            :options="kindOptions"
          />
          <label
            v-if="kind === 'cloud'"
            class="flex flex-col gap-1.5 text-chrome text-fg-muted"
          >
            Project name
            <TextInput
              v-model="name"
              focused
              maxlength="64"
              placeholder="My next idea"
            />
          </label>
          <template v-else>
            <div class="flex flex-col gap-1.5 text-chrome text-fg-muted">
              Device
              <span class="flex items-center gap-2">
                <!-- The menu fills the row, the way the file browser's device picker does; Add device sits after it. -->
                <Dropdown
                  :overlay-store="overlayStore"
                  class="min-w-0 flex-1 [&>div]:w-full"
                >
                  <template #trigger="{ isOpen }">
                    <span
                      role="button"
                      aria-label="Device"
                      class="flex h-7 w-full cursor-default select-none items-center gap-2 rounded-md px-2 text-chrome text-fg transition-colors duration-200 ease-out"
                      :class="isOpen ? 'bg-active' : 'bg-hover hover:bg-active'"
                    >
                      <component
                        :is="hostIcon({ id: deviceId })"
                        :size="ICON_PX.in28"
                        class="shrink-0 text-fg-muted"
                      />
                      <span class="min-w-0 flex-1 truncate">{{ deviceLabel }}</span>
                      <ChevronDown
                        :size="ICON_PX.in24"
                        class="shrink-0 text-fg-subtle transition-transform duration-200 ease-out"
                        :class="isOpen ? 'rotate-180' : ''"
                      />
                    </span>
                  </template>
                  <template #content="{ close, triggerWidth }">
                    <Menu :style="{ minWidth: `${triggerWidth}px` }">
                      <MenuItem
                        v-for="entry in devices"
                        :key="entry.id"
                        :icon="hostIcon(entry)"
                        :label="entry.name"
                        :indicator="entry.online ? 'success' : 'muted'"
                        :indicator-label="entry.online ? 'Online' : 'Offline'"
                        :note="entry.online ? undefined : 'offline'"
                        :disabled="!entry.online"
                        disabled-reason="This device is offline."
                        choice
                        :is-selected="deviceId === entry.id"
                        @select="deviceId = entry.id; close()"
                      />
                      <div
                        v-if="!devices.length"
                        class="select-none px-2 py-3 text-center text-chrome text-fg-subtle"
                      >No devices yet.</div>
                    </Menu>
                  </template>
                </Dropdown>
                <Button class="shrink-0" @click="emit('connectDevice')">
                  <Plus :size="14" />
                  Add device
                </Button>
              </span>
            </div>
            <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
              Directory
              <span class="flex items-center gap-2">
                <TextInput
                  v-model="path"
                  placeholder="/path/to/project"
                  class="min-w-0 flex-1"
                />
                <Button
                  class="shrink-0"
                  :disabled="!online"
                  @click="browsing = true"
                >
                  <FolderOpen :size="14" />
                  Browse…
                </Button>
              </span>
              <span class="text-[12px] leading-4 text-fg-subtle">{{ projectName ? `The project will be called ${projectName}.` : 'The project takes the folder\'s name.' }}</span>
            </label>
          </template>
          <InlineError v-if="message" :message="message" />
          <div>
            <Button
              variant="primary"
              :disabled="!canCreate"
              @click="create"
            >Create project</Button>
          </div>
        </form>
      </div>
    </template>
  </Dialog>
</template>
