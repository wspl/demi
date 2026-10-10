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
import DeviceIcon from '../devices/DeviceIcon.vue'
import { DEVICE_GLYPHS } from '../devices/device-glyphs'
import HostChoices from './HostChoices.vue'
import { MOVE_LOCKED } from './move-question'
import type { HostChoice, HostDeviceOption, HostMenuHost } from './types'

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
  choose: [host: HostChoice]
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


function choose(host: HostChoice) {
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
          <HostChoices
            :primary-host="primaryHost"
            :devices="devices"
            :choose-directory="chooseDirectory"
            :locked="locked ? MOVE_LOCKED : undefined"
            @choose="choose"
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
            :disabled-reason="MOVE_LOCKED"
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
