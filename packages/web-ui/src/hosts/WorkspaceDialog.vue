<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import { computed, ref, useId, watch } from 'vue'
import { FolderOpen, Plus, X } from '@lucide/vue'
import { hostIcon } from './icons'
import { baseName } from '@demicodes/utils'
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '../ui/Button.vue'
import { operations, type Action, type Operation } from '../ui/operations'
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
import { deviceGlyph } from '../devices/device-glyphs'
import { DEVICE_STATE_LABEL } from '../devices/state'
import {
  openingChoice,
  useDeviceProjectAction,
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
 * the Cloud the first time (`conversations.md` § Conversations and projects); the
 * host remembers it from `choose`. Moving a conversation between existing
 * projects is the conversation header's menus, not this dialog.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  load?: 'loading' | 'ready' | 'failed'
  devices: WorkspaceDevice[]
  /** The user's last choice, which the form opens on; absent the first time. */
  lastHost?: WorkspaceHostChoice
  /** What went wrong with the last Create, shown under the form. */
  message?: ShownError | null
  /** The file browser's tree for a device. */
  sourceFor: (deviceId: string) => FileBrowserSource
  placesFor?: (deviceId: string) => FileBrowserPlaceGroup[]
  /** Creates the project; the button works, and the form waits, until it settles. */
  onCreate?: Action<[draft: WorkspaceDraft]>
}>()

/** Creating a project (`operations.ts`): the form opened again while it works shows it working. */
const CREATING: Operation = { action: 'create', target: 'project:new' }
const creating = computed(() => operations.runs(CREATING))

const emit = defineEmits<{
  retry: []
  close: []
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
    get label() {
      return uiText('common.cloud')
    },
    get description() {
      return t('project.cloudDescription')
    },
    icon: hostIcon({ kind: 'cloud' }),
  },
  {
    value: 'device',
    get label() {
      return t('project.device')
    },
    get description() {
      return t('project.deviceDescription')
    },
    icon: hostIcon({ kind: 'device' }),
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
const deviceLabel = computed(() => device.value?.name ?? t('project.chooseDevice'))
const browserHosts = computed(() =>
  props.devices.map((entry) => ({
    id: entry.id,
    label: entry.name,
    state: entry.state,
    mark: entry.mark,
  })),
)
const browserSource = computed(() => props.sourceFor(deviceId.value))
const browserPlaces = computed(() => props.placesFor?.(deviceId.value) ?? [])
const online = computed(() => device.value?.state === 'online')
/**
 * What the button says: a device's directory is added when it exists, else
 * created; a Cloud project is always new. Only a shown device form keeps the
 * directory's listing.
 */
const deviceAction = useDeviceProjectAction({
  source: () => browserSource.value,
  path: () => (props.isOpen && kind.value === 'device' ? path.value : ''),
})
const action = computed(() => (kind.value === 'cloud' || deviceAction.value === 'create' ? t('project.create') : t('project.add')))
const directoryId = useId()
/** What a device project will be called: the directory's name. */
const projectName = computed(() => baseName(path.value))
const canCreate = computed(
  () =>
    !creating.value &&
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
    if (!shown || creating.value || props.message) {
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

function create(): unknown {
  if (!canCreate.value) {
    return
  }
  return props.onCreate?.(
    kind.value === 'cloud'
      ? { kind: 'cloud', name: name.value.trim() }
      : { kind: 'device', deviceId: deviceId.value, path: path.value.replace(/\/$/, '') || '/' },
  )
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
    :label="t('project.title')"
    hide-close
    @close="emit('close')"
  >
    <div v-if="browsing" class="flex h-[32rem] min-h-0 flex-col">
      <header
        class="flex h-11 shrink-0 select-none items-center justify-between gap-2 border-b border-line pl-4 pr-2"
      >
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('project.selectFolder') }}</h3>
        <IconButton
          :icon="X"
          variant="ghost"
          :aria-label="uiText('common.close')"
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
      <!-- The close keeps the same 12 px to the bar's top, bottom and trailing edge; the title lines up with the form. -->
      <header
        class="flex select-none items-center justify-between border-b border-line py-3 pl-5 pr-3"
      >
        <h2 class="text-[15px] font-medium text-fg-emphasis">{{ t('project.title') }}</h2>
        <IconButton
          :icon="X"
          variant="ghost"
          :aria-label="uiText('common.close')"
          @click="emit('close')"
        />
      </header>
      <AsyncRegion
        :state="load"
        :label="t('project.loading')"
        @retry="emit('retry')"
      >
        <div class="flex flex-col gap-4 px-5 pt-4">
          <ChoiceCards
            :model-value="kind"
            :options="kindOptions"
            @update:model-value="chooseKind"
          />
          <label
            v-if="kind === 'cloud'"
            class="flex flex-col gap-1.5 text-chrome text-fg-muted"
          >
            {{ t('project.name') }}
            <TextInput
              v-model="name"
              :disabled="creating"
              focused
              maxlength="64"
            />
          </label>
          <template v-else>
            <div class="flex flex-col gap-1.5 text-chrome text-fg-muted">
              {{ t('project.device') }}
              <span class="flex items-center gap-2">
                <!-- The menu fills the row, the way the file browser's device picker does; Add Device sits after it. -->
                <Dropdown
                  :overlay-store="overlayStore"
                  :disabled="creating"
                  variant="field"
                  width="fill"
                  :trigger-label="t('project.device')"
                  :icon="device ? deviceGlyph(device.state, device.mark) : deviceGlyph(null, null)"
                  class="flex-1"
                >
                  <template #trigger>
                    <TruncatedText class="flex-1" :text="deviceLabel" />
                  </template>
                  <template #content="{ close, triggerWidth }">
                    <Menu :style="{ minWidth: `${triggerWidth}px` }">
                      <MenuItem
                        v-for="entry in devices"
                        :key="entry.id"
                        :icon="deviceGlyph(entry.state, entry.mark)"
                        :label="entry.name"
                        :value="entry.state === 'online' ? undefined : DEVICE_STATE_LABEL[entry.state]"
                        :disabled="entry.state !== 'online'"
                        :disabled-reason="t('choices.offline')"
                        choice
                        :is-selected="deviceId === entry.id"
                        @select="selectDevice(entry.id, close)"
                      />
                      <div
                        v-if="!devices.length"
                        class="select-none px-2 py-3 text-center text-chrome text-fg-subtle"
                      >
                        {{ t('project.noDevices') }}
                      </div>
                    </Menu>
                  </template>
                </Dropdown>
                <Button
                  class="shrink-0"
                  :disabled="creating"
                  @click="emit('connectDevice', paired)"
                >
                  <Plus :size="14" />
                  {{ t('project.addDevice') }}
                </Button>
              </span>
            </div>
            <!-- Not a <label>: a label hands every click inside it, on the caption, the hint or
                 the gaps between, to the field, which would take the focus back and reopen
                 its completion (`conversations.md` § Conversations and projects). -->
            <div class="flex flex-col gap-1.5 text-chrome text-fg-muted">
              <span :id="`${directoryId}-label`">{{ t('project.directory') }}</span>
              <span class="flex items-center gap-2">
                <!-- Completes from the device's folders as it is typed; Return outside the completion creates. -->
                <PathInput
                  v-model="path"
                  :source="browserSource"
                  kind="directory"
                  :disabled="creating"
                  placeholder="/path/to/project"
                  :aria-labelledby="`${directoryId}-label`"
                  :aria-describedby="`${directoryId}-hint`"
                  class="min-w-0 flex-1"
                />
                <Button
                  class="shrink-0"
                  :disabled="!online || creating"
                  @click="browsing = true"
                >
                  <FolderOpen :size="14" />
                  {{ t('project.browse') }}
                </Button>
              </span>
              <span :id="`${directoryId}-hint`" class="text-detail leading-4 text-fg-subtle">{{
                projectName
                  ? t('project.called', { name: projectName })
                  : t('project.takesName')
              }}</span>
            </div>
          </template>
          <InlineError v-if="message" :message="message.message" :request-id="message.requestId" />
        </div>
      </AsyncRegion>
    </template>
    <template v-if="!browsing && (load ?? 'ready') === 'ready'" #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button
        variant="primary"
        :disabled="!canCreate && !creating"
        :operation="CREATING"
        @click="create"
        >{{ action }}</Button
      >
    </template>
  </Dialog>
</template>
