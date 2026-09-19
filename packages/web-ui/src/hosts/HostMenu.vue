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
import HostPicker from './HostPicker.vue'
import type { HostDeviceOption, HostMenuHost } from './types'

const props = defineProps<{
  mainHost: HostMenuHost
  attachedHosts: HostMenuHost[]
  devices: HostDeviceOption[]
  pending?: boolean
  mainLocked?: boolean
  attachmentsLocked?: boolean
}>()
const emit = defineEmits<{
  switchMain: [id: string]
  attach: [id: string]
  detach: [id: string]
  connect: []
}>()

const open = ref(false)
const boundIds = computed(() => [
  props.mainHost.id,
  ...props.attachedHosts.map((host) => host.id),
])

function selectMain(id: string) {
  if (props.mainLocked) {
    return
  }
  open.value = false
  emit('switchMain', id)
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
      <Button
        variant="ghost"
        class="max-w-full"
        aria-label="Manage conversation hosts"
        :loading="pending"
      >
        <span class="relative flex shrink-0">
          <component
            :is="mainHost.kind === 'cloud' ? Cloud : Monitor"
            :size="ICON_PX.in28"
          />
          <CornerDot
            v-if="mainHost.kind === 'device'"
            :tone="mainHost.online ? 'success' : 'muted'"
            ring="button"
            :label="mainHost.online ? 'Online' : 'Offline'"
          />
        </span>
        <span class="max-w-28 truncate">{{ mainHost.name }}</span>
        <span v-if="attachedHosts.length" class="text-[11px] text-fg-subtle">
          +{{ attachedHosts.length }}
        </span>
      </Button>
    </template>
    <template #content>
      <Menu>
        <MenuItem
          :icon="mainHost.kind === 'cloud' ? Cloud : Monitor"
          label="Main host"
          :indicator="mainHost.kind === 'cloud' ? undefined : mainHost.online ? 'success' : 'muted'"
          :indicator-label="mainHost.online ? 'Online' : 'Offline'"
          :value="mainHost.name"
          :disabled="mainLocked"
          has-submenu
        >
          <template #submenu>
            <HostPicker
              :devices="devices"
              include-cloud
              :selected-id="mainHost.id"
              @select="selectMain"
              @connect="connect"
            />
          </template>
        </MenuItem>
        <MenuGroup v-if="attachedHosts.length" label="Attached hosts">
          <MenuItem
            v-for="host in attachedHosts"
            :key="host.id"
            :icon="host.kind === 'cloud' ? Cloud : Monitor"
            :label="host.name"
            :indicator="host.kind === 'cloud' ? undefined : host.online ? 'success' : 'muted'"
            :indicator-label="host.online ? 'Online' : 'Offline'"
            :note="host.kind === 'device' && !host.online ? 'offline' : undefined"
            has-submenu
          >
            <template #submenu>
              <Menu>
                <MenuItem
                  label="Use as main environment…"
                  :icon="host.kind === 'cloud' ? Cloud : Monitor"
                  :disabled="mainLocked || (host.kind === 'device' && !host.online)"
                  @select="selectMain(host.id)"
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
          label="Attach device…"
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
        <MenuItem label="Connect new device…" :icon="Link" @select="connect" />
      </Menu>
    </template>
  </Dropdown>
</template>
