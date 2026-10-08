<script setup lang="ts">
import { ref } from 'vue'
import { Cloud, Plus, Unlink } from '@lucide/vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { ICON_PX } from '../ui/icon-metrics'
import { COMPACT_LABEL_CLASS, useRoomLabel } from '../ui/label-room'
import Tooltip from '../ui/Tooltip.vue'
import { isTextCut } from '../ui/truncation'
import type { SentenceText } from '../ui/ui-text'
import DeviceIcon from '../devices/DeviceIcon.vue'
import { DEVICE_GLYPHS } from '../devices/device-glyphs'
import { DEVICE_STATE_LABEL } from '../devices/state'
import { HOST_PATH_LABEL, type HostDeviceOption, type HostMenuHost } from './types'

/**
 * Where a conversation runs, as the header shows it and its one-level menu
 * changes it (`product.md` § Where a conversation runs): Run On lists the
 * Cloud and every paired device, each online device with the path this page
 * reaches it by, the primary Host checked; Attached lists the devices the
 * conversation's agents attached, each with Detach. Choosing a Host is the
 * host's to carry out: a conversation outside a project moves there, one in
 * a project opens that device's directory picker, which `chooseDirectory`
 * marks with an ellipsis on each device but the checked one. The Cloud
 * lists no directories to a page, so it moves the conversation to its own
 * directory there and never opens a picker.
 */
const props = defineProps<{
  primaryHost: HostMenuHost
  /** The user's paired devices, which Run On lists after the Cloud. */
  devices: HostDeviceOption[]
  attachedHosts: HostMenuHost[]
  /** In a project: choosing a Host opens its directory picker. */
  chooseDirectory?: boolean
  /** The conversation's work runs, or it is archived: nothing here can change it. */
  locked?: boolean
  pending?: boolean
}>()
const emit = defineEmits<{
  /** A Host chosen to run on: the Cloud, or a device by its id. */
  choose: [host: { kind: 'cloud' } | { kind: 'device'; id: string }]
  detach: [id: string]
  connect: []
}>()

const open = ref(false)
// Where the header's title needs the width, the host is its icon and its name a tooltip.
const hostLabel = ref<HTMLElement | null>(null)
const hostCompact = useRoomLabel(hostLabel, () => props.primaryHost.name, 0)

/** The name is in the tooltip while the button hides it or cuts it. */
function nameHidden(): boolean {
  return hostCompact.value || (hostLabel.value != null && isTextCut(hostLabel.value))
}

const LOCKED: SentenceText = 'This conversation can move once its work ends.'

/** A device's name in Run On: with an ellipsis where choosing it opens a picker first. */
function choiceLabel(device: HostDeviceOption): string {
  const opensPicker = props.chooseDirectory && device.state === 'online' && !deviceChecked(device.id)
  return opensPicker ? `${device.name}…` : device.name
}

/** The end of a device's row: the path this page reaches it by while it is online, otherwise its state. */
function deviceNote(device: HostDeviceOption): string | undefined {
  if (device.state !== 'online')
    return DEVICE_STATE_LABEL[device.state]
  return device.path ? HOST_PATH_LABEL[device.path] : undefined
}

function deviceChecked(id: string): boolean {
  return props.primaryHost.kind === 'device' && props.primaryHost.id === id
}

function choose(host: { kind: 'cloud' } | { kind: 'device'; id: string }) {
  if (props.locked)
    return
  open.value = false
  emit('choose', host)
}

/**
 * An attached Host chosen to run on, as in Run On. An offline device is
 * drawn faded and cannot be chosen, but its row stays enabled, since Detach
 * takes it back whatever its state.
 */
function chooseAttached(host: HostMenuHost) {
  if (host.kind === 'cloud') {
    choose({ kind: 'cloud' })
  } else if (host.state === 'online') {
    choose({ kind: 'device', id: host.id })
  }
}

function detach(id: string) {
  open.value = false
  emit('detach', id)
}

function connect() {
  open.value = false
  emit('connect')
}
</script>

<template>
  <Dropdown
    v-model:open="open"
    :overlay-store="appOverlayStore"
    :disabled="pending"
    width="shrink"
  >
    <template #trigger>
      <Tooltip :content="primaryHost.name" :show-if="nameHidden">
        <Button
          variant="ghost"
          class="max-w-full"
          aria-label="Where this conversation runs"
          :loading="pending"
        >
          <Cloud v-if="primaryHost.kind === 'cloud'" :size="ICON_PX.in28" class="shrink-0" />
          <DeviceIcon v-else :state="primaryHost.state" />
          <span
            ref="hostLabel"
            class="max-w-28 truncate"
            :class="hostCompact ? COMPACT_LABEL_CLASS : ''"
          >{{ primaryHost.name }}</span>
          <span v-if="attachedHosts.length" class="text-[11px] text-fg-subtle">
            +{{ attachedHosts.length }}
          </span>
        </Button>
      </Tooltip>
    </template>
    <template #content>
      <Menu class="max-w-80">
        <MenuGroup label="Run On">
          <MenuItem
            :icon="Cloud"
            label="Cloud"
            choice
            :is-selected="primaryHost.kind === 'cloud'"
            :disabled="locked"
            :disabled-reason="LOCKED"
            @select="choose({ kind: 'cloud' })"
          />
          <MenuItem
            v-for="device in devices"
            :key="device.id"
            :icon="DEVICE_GLYPHS[device.state]"
            :label="choiceLabel(device)"
            :value="deviceNote(device)"
            choice
            :is-selected="deviceChecked(device.id)"
            :disabled="locked || device.state !== 'online'"
            :disabled-reason="locked ? LOCKED : 'This device is offline.'"
            @select="choose({ kind: 'device', id: device.id })"
          />
          <MenuItem label="Add Device…" :icon="Plus" @select="connect" />
        </MenuGroup>
        <MenuGroup v-if="attachedHosts.length" label="Attached">
          <MenuItem
            v-for="host in attachedHosts"
            :key="host.id"
            :icon="host.kind === 'cloud' ? Cloud : DEVICE_GLYPHS[host.state]"
            :label="host.name"
            :faded="host.kind === 'device' && host.state !== 'online'"
            :disabled="locked"
            :disabled-reason="LOCKED"
            @select="chooseAttached(host)"
          >
            <template #actions>
              <Tooltip content="Detach" class="inline-flex">
                <IconButton
                  :icon="Unlink"
                  variant="ghost"
                  size="xs"
                  aria-label="Detach"
                  :disabled="locked"
                  @click.stop="detach(host.id)"
                />
              </Tooltip>
            </template>
          </MenuItem>
        </MenuGroup>
      </Menu>
    </template>
  </Dropdown>
</template>
