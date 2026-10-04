<script setup lang="ts">
import { computed, ref } from 'vue'
import { Cloud, Link, Monitor, Plus, Unlink } from '@lucide/vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import Button from '../ui/Button.vue'
import CornerDot from '../ui/CornerDot.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { ICON_PX } from '../ui/icon-metrics'
import { COMPACT_LABEL_CLASS, useRoomLabel } from '../ui/label-room'
import Tooltip from '../ui/Tooltip.vue'
import { isTextCut } from '../ui/truncation'
import HostPicker from './HostPicker.vue'
import type { HostDeviceOption, HostMenuHost } from './types'

const props = defineProps<{
  primaryHost: HostMenuHost
  attachedHosts: HostMenuHost[]
  devices: HostDeviceOption[]
  pending?: boolean
  primaryLocked?: boolean
  attachmentsLocked?: boolean
}>()
const emit = defineEmits<{
  switchPrimary: [id: string]
  attach: [id: string]
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
const boundIds = computed(() => [
  props.primaryHost.id,
  ...props.attachedHosts.map((host) => host.id),
])

function selectPrimary(id: string) {
  if (props.primaryLocked) {
    return
  }
  open.value = false
  emit('switchPrimary', id)
}

function attach(id: string) {
  open.value = false
  emit('attach', id)
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
          aria-label="Manage conversation hosts"
          :loading="pending"
        >
          <span class="relative flex shrink-0">
            <component
              :is="primaryHost.kind === 'cloud' ? Cloud : Monitor"
              :size="ICON_PX.in28"
            />
            <CornerDot
              v-if="primaryHost.kind === 'device'"
              :tone="primaryHost.online ? 'success' : 'muted'"
              ring="button"
              :label="primaryHost.online ? 'Online' : 'Offline'"
            />
          </span>
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
      <Menu>
        <MenuItem
          :icon="primaryHost.kind === 'cloud' ? Cloud : Monitor"
          label="Primary Host"
          :indicator="primaryHost.kind === 'cloud' ? undefined : primaryHost.online ? 'success' : 'muted'"
          :indicator-label="primaryHost.online ? 'Online' : 'Offline'"
          :value="primaryHost.name"
          :disabled="primaryLocked"
          has-submenu
        >
          <template #submenu>
            <HostPicker
              :devices="devices"
              include-cloud
              :selected-id="primaryHost.id"
              @select="selectPrimary"
              @connect="connect"
            />
          </template>
        </MenuItem>
        <MenuGroup v-if="attachedHosts.length" label="Attached Hosts">
          <MenuItem
            v-for="host in attachedHosts"
            :key="host.id"
            :icon="host.kind === 'cloud' ? Cloud : Monitor"
            :label="host.name"
            :indicator="host.kind === 'cloud' ? undefined : host.online ? 'success' : 'muted'"
            :indicator-label="host.online ? 'Online' : 'Offline'"
            :note="host.kind === 'device' && !host.online ? 'Offline' : undefined"
            has-submenu
          >
            <template #submenu>
              <Menu>
                <MenuItem
                  label="Use as Primary Environment…"
                  :icon="host.kind === 'cloud' ? Cloud : Monitor"
                  :disabled="primaryLocked || (host.kind === 'device' && !host.online)"
                  @select="selectPrimary(host.id)"
                />

                <MenuItem
                  label="Detach"
                  :icon="Unlink"
                  :disabled="attachmentsLocked"
                  @select="detach(host.id)"
                />
              </Menu>
            </template>
          </MenuItem>
        </MenuGroup>
        <MenuDivider />
        <MenuItem
          label="Attach Device"
          :icon="Plus"
          has-submenu
          :disabled="attachmentsLocked"
        >
          <template #submenu>
            <HostPicker
              :devices="devices"
              :bound-ids="boundIds"
              @select="attach"
              @connect="connect"
            />
          </template>
        </MenuItem>
        <MenuItem label="Connect New Device…" :icon="Link" @select="connect" />
      </Menu>
    </template>
  </Dropdown>
</template>
