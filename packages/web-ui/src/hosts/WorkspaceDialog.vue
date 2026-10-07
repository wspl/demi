<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Cloud, FolderOpen, Monitor, Plus, X } from '@lucide/vue'
import { hostIcon } from './icons'
import { ICON_PX } from '../ui/icon-metrics'
import { baseName } from '../files/paths'
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '../ui/Button.vue'
import ChoiceCards, { type ChoiceCardOption } from '../ui/ChoiceCards.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import TextInput from '../ui/TextInput.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import InlineError from '../ui/InlineError.vue'
import FileBrowser from '../files/FileBrowser.vue'
import PathInput from '../files/PathInput.vue'
import type { FileBrowserPlaceGroup, FileBrowserSource } from '../files/types'
import type { PairingDevice } from '../devices/pairing'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE } from '../devices/state'
import {
  openingChoice,
  usePairedSelection,
  type WorkspaceDevice,
  type WorkspaceDraft,
  type WorkspaceHostChoice,
} from './workspace'

/**
 * A new project for a conversation to work in. The form branches on where it
 * lives: on the Cloud the workspace is managed and only needs a name; on a
 * device it is a directory there, and takes the directory's name. The folder
 * browser is a page of the same dialog, opened by Browse…. Every opening
 * starts from a clean form on the user's last choice of kind and device,
 * the Cloud the first time (`product.md` § Conversations and projects); the
 * host remembers it from `choose`. Switching between existing projects is the
 * sidebar's Move To and the conversation header's workspace control, not this
 * dialog.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  pending?: boolean
  load?: 'loading' | 'ready' | 'failed'
  devices: WorkspaceDevice[]
  /** The user's last choice, which the form opens on; absent the first time. */
  lastHost?: WorkspaceHostChoice
  /** What went wrong with the last Create, shown under the form. */
  message?: string
  /** The file browser's tree for a device. */
  sourceFor: (deviceId: string) => FileBrowserSource
  placesFor?: (deviceId: string) => FileBrowserPlaceGroup[]
}>()

const emit = defineEmits<{
  retry: []
  close: []
  create: [draft: WorkspaceDraft]
  /** The user chose a kind or a device: the choice the next opening starts on. */
  choose: [choice: WorkspaceHostChoice]
  /**
   * The Add Device button beside the device menu: the host starts pairing and
   * hands `onPaired` the device it pairs, which the menu then selects.
   */
  connectDevice: [onPaired: (device: PairingDevice) => void]
}>()

type Kind = 'cloud' | 'device'
const kindOptions = [
  {
    value: 'cloud',
    label: 'Cloud',
    description: 'A managed workspace, ready at once.',
    icon: Cloud,
  },
  {
    value: 'device',
    label: 'Device',
    description: 'A directory on one of your devices.',
    icon: Monitor,
  },
] as const satisfies readonly ChoiceCardOption<Kind>[]

const opening = openingChoice(props.lastHost, props.devices)
const kind = ref<Kind>(opening.kind)
const name = ref('')
const path = ref('')
const deviceId = ref(opening.deviceId ?? '')
const browsing = ref(false)

const device = computed(
  () => props.devices.find((entry) => entry.id === deviceId.value) ?? null,
)
const deviceLabel = computed(() => device.value?.name ?? 'Choose a device')
const browserHosts = computed(() =>
  props.devices.map((entry) => ({
    id: entry.id,
    label: entry.name,
    state: entry.state,
  })),
)
const browserSource = computed(() => props.sourceFor(deviceId.value))
const browserPlaces = computed(() => props.placesFor?.(deviceId.value) ?? [])
const online = computed(() => device.value?.state === 'online')
/** What a device project will be called: the directory's name. */
const projectName = computed(() => baseName(path.value.replace(/\/$/, '')))
const canCreate = computed(
  () =>
    !props.pending &&
    (kind.value === 'cloud'
      ? !!name.value.trim()
      : online.value && path.value.startsWith('/') && !!projectName.value),
)

// The form starts again each time it shows, after the devices have loaded,
// so a remembered device is found among them.
watch(
  () => props.isOpen && (props.load ?? 'ready') === 'ready',
  (shown) => {
    // A request in flight or its failure keeps the form's input.
    if (!shown || props.pending || props.message) {
      return
    }
    const choice = openingChoice(props.lastHost, props.devices)
    kind.value = choice.kind
    deviceId.value = choice.deviceId ?? ''
    path.value = homeOf(deviceId.value)
    name.value = ''
    browsing.value = false
  },
)

// The directory starts at the chosen device's home; picking another device starts over there.
watch(
  deviceId,
  (id) => {
    path.value = homeOf(id)
  },
  { immediate: true },
)

function homeOf(id: string): string {
  return id ? props.sourceFor(id).home : ''
}

function pickDirectory(chosen: string) {
  path.value = chosen
  browsing.value = false
}

function create() {
  if (!canCreate.value) {
    return
  }
  if (kind.value === 'cloud') {
    emit('create', {
      kind: 'cloud',
      name: name.value.trim(),
    })
  } else {
    emit('create', {
      kind: 'device',
      deviceId: deviceId.value,
      path: path.value.replace(/\/$/, '') || '/',
    })
  }
}
function choose(): void {
  emit('choose', {
    kind: kind.value,
    ...(deviceId.value ? { deviceId: deviceId.value } : {}),
  })
}
function chooseKind(chosen: Kind): void {
  kind.value = chosen
  choose()
}
function chooseDevice(id: string): void {
  deviceId.value = id
  choose()
}
// A device paired from Add Device is chosen here once the list shows it.
const paired = usePairedSelection(() => props.devices, chooseDevice)
function selectDevice(id: string, close: () => void): void {
  chooseDevice(id)
  close()
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :size="browsing ? 'lg' : 'md'"
    label="New Project"
    hide-close
    @close="emit('close')"
  >
    <div v-if="browsing" class="flex h-[32rem] min-h-0 flex-col">
      <header
        class="flex h-11 shrink-0 select-none items-center justify-between gap-2 border-b border-line pl-4 pr-2"
      >
        <h3 class="text-[15px] font-medium text-fg-emphasis">Select Folder</h3>
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
        :initial-path="
          path.startsWith('/') ? path.replace(/\/$/, '') || '/' : undefined
        "
        :places="browserPlaces"
        :hosts="browserHosts"
        :host-id="deviceId"
        @select="pickDirectory"
        @cancel="browsing = false"
        @update:host-id="chooseDevice"
      />
    </div>
    <template v-else>
      <header
        class="flex select-none items-center justify-between border-b border-line px-4 py-3"
      >
        <h2 class="text-[15px] font-medium text-fg-emphasis">New Project</h2>
        <IconButton
          :icon="X"
          variant="ghost"
          aria-label="Close"
          @click="emit('close')"
        />
      </header>
      <AsyncRegion
        :state="load"
        label="Loading devices…"
        @retry="emit('retry')"
      >
        <div class="flex flex-col gap-4 p-4">
          <form class="flex flex-col gap-4" @submit.prevent="create">
            <ChoiceCards
              :model-value="kind"
              :options="kindOptions"
              @update:model-value="chooseKind"
            />
            <label
              v-if="kind === 'cloud'"
              class="flex flex-col gap-1.5 text-chrome text-fg-muted"
            >
              Project name
              <TextInput
                v-model="name"
                :disabled="pending"
                focused
                maxlength="64"
                placeholder="My next idea"
              />
            </label>
            <template v-else>
              <div class="flex flex-col gap-1.5 text-chrome text-fg-muted">
                Device
                <span class="flex items-center gap-2">
                  <!-- The menu fills the row, the way the file browser's device picker does; Add Device sits after it. -->
                  <Dropdown
                    :overlay-store="overlayStore"
                    :disabled="pending"
                    variant="field"
                    width="fill"
                    trigger-label="Device"
                    class="flex-1"
                  >
                    <template #trigger>
                      <component
                        :is="hostIcon({ id: deviceId })"
                        :size="ICON_PX.in28"
                        class="shrink-0 text-fg-muted"
                      />
                      <TruncatedText class="flex-1" :text="deviceLabel" />
                    </template>
                    <template #content="{ close, triggerWidth }">
                      <Menu :style="{ minWidth: `${triggerWidth}px` }">
                        <MenuItem
                          v-for="entry in devices"
                          :key="entry.id"
                          :icon="hostIcon(entry)"
                          :label="entry.name"
                          :indicator="DEVICE_STATE_TONE[entry.state]"
                          :indicator-label="DEVICE_STATE_LABEL[entry.state]"
                          :note="entry.state === 'online' ? undefined : DEVICE_STATE_LABEL[entry.state]"
                          :disabled="entry.state !== 'online'"
                          disabled-reason="This device is offline."
                          choice
                          :is-selected="deviceId === entry.id"
                          @select="selectDevice(entry.id, close)"
                        />
                        <div
                          v-if="!devices.length"
                          class="select-none px-2 py-3 text-center text-chrome text-fg-subtle"
                        >
                          No devices yet.
                        </div>
                      </Menu>
                    </template>
                  </Dropdown>
                  <Button
                    class="shrink-0"
                    :disabled="pending"
                    @click="emit('connectDevice', paired)"
                  >
                    <Plus :size="14" />
                    Add Device
                  </Button>
                </span>
              </div>
              <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
                Directory
                <span class="flex items-center gap-2">
                  <!-- Completes from the device's folders as it is typed; Enter still creates. -->
                  <PathInput
                    v-model="path"
                    :source="browserSource"
                    kind="directory"
                    :disabled="pending"
                    placeholder="/path/to/project"
                    class="min-w-0 flex-1"
                  />
                  <Button
                    class="shrink-0"
                    :disabled="!online || pending"
                    @click="browsing = true"
                  >
                    <FolderOpen :size="14" />
                    Browse…
                  </Button>
                </span>
                <span class="text-[12px] leading-4 text-fg-subtle">{{
                  projectName
                    ? `The project will be called ${projectName}.`
                    : 'The project takes the folder’s name.'
                }}</span>
              </label>
            </template>
            <InlineError v-if="message" :message="message" />
            <div>
              <Button
                variant="primary"
                :disabled="!canCreate && !pending"
                :loading="pending"
                @click="create"
                >Create Project</Button
              >
            </div>
          </form>
        </div>
      </AsyncRegion>
    </template>
  </Dialog>
</template>
