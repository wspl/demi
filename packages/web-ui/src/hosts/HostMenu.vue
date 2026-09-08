<script setup lang="ts">
import { computed, ref } from 'vue'
import { Cloud, Link, Monitor, Plus, Unlink } from '@lucide/vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import Button from '../ui/Button.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { ICON_PX } from '../ui/icon-metrics'
import HostPicker from './HostPicker.vue'
import type { HostDeviceOption, HostMenuMainHost } from './types'

const props = defineProps<{
  mainHost: HostMenuMainHost
  attachedHosts: HostDeviceOption[]
  devices: HostDeviceOption[]
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
  ...props.attachedHosts.map(host => host.id),
])

function selectMain(id: string) {
  if (props.mainLocked) return
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
  <Dropdown v-model:open="open" :overlay-store="appOverlayStore" class="min-w-0 [&>div]:min-w-0">
    <template #trigger>
      <Button variant="ghost" class="max-w-full" aria-label="Manage conversation hosts">
        <component
          :is="mainHost.kind === 'cloud' ? Cloud : Monitor"
          :size="ICON_PX.in28"
          class="shrink-0"
        />
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
            :icon="Monitor"
            :label="host.name"
            :indicator="host.online ? 'success' : 'muted'"
            :indicator-label="host.online ? 'Online' : 'Offline'"
            :note="host.online ? undefined : 'offline'"
            has-submenu
          >
            <template #submenu>
              <Menu>
                <MenuItem
                  label="Use as main environment…"
                  :icon="Monitor"
                  :disabled="mainLocked || !host.online"
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
        <MenuItem label="Attach device…" :icon="Plus" has-submenu :disabled="attachmentsLocked">
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
