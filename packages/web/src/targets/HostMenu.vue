<script setup lang="ts">
import { computed, ref } from 'vue'
import { Cloud, Link, Monitor, Plus, Unlink } from '@lucide/vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import HostPicker from './HostPicker.vue'
import type { Conversation, Project } from '../prototype/types'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'

const props = defineProps<{ conversation: Conversation; project?: Project }>()
const emit = defineEmits<{ switchMain: [deviceId: string, cwd?: string] }>()
const resources = useResources()
const store = useConversations()
const open = ref(false)
const mainLocked = computed(() => !!props.conversation.stream || props.conversation.archived)
const boundIds = computed(() => [
  ...(props.project ? [props.project.deviceId] : []),
  ...props.conversation.attachedHosts.map((host) => host.deviceId),
])
function isOnline(deviceId: string) {
  return (
    deviceId === 'cloud' || !!resources.devices.find((device) => device.id === deviceId)?.online
  )
}
function switchMain(deviceId: string, cwd?: string) {
  if (mainLocked.value) return
  open.value = false
  emit('switchMain', deviceId, cwd)
}
function selectMain(id: string) {
  switchMain(id, props.conversation.attachedHosts.find((host) => host.deviceId === id)?.cwd)
}
function attach(id: string) {
  store.attachHost(props.conversation, id)
  open.value = false
}
function detach(id: string) {
  store.detachHost(props.conversation, id)
  open.value = false
}
function connect() {
  open.value = false
  resources.settingsTab = 'Devices'
  resources.settingsOpen = true
}
</script>

<template>
  <Dropdown v-model:open="open" :overlay-store="appOverlayStore" class="min-w-0 [&>div]:min-w-0">
    <template #trigger>
      <Button variant="ghost" class="max-w-full" aria-label="Manage conversation hosts">
        <component
          :is="!project || project.hostKind === 'cloud' ? Cloud : Monitor"
          :size="ICON_PX.in28"
          class="shrink-0"
        />
        <span v-if="project?.hostKind === 'device'" class="max-w-28 truncate">
          {{ project.host }}
        </span>
        <span v-if="conversation.attachedHosts.length" class="text-[11px] text-fg-subtle">
          +{{ conversation.attachedHosts.length }}
        </span>
      </Button>
    </template>
    <template #content>
      <Menu class="w-80 max-w-[calc(100vw-2rem)]">
        <MenuItem
          :icon="!project || project.hostKind === 'cloud' ? Cloud : Monitor"
          label="Main host"
          :value="project?.host ?? 'Cloud'"
          :disabled="mainLocked"
          has-submenu
        >
          <template #submenu>
            <HostPicker
              :devices="resources.devices"
              include-cloud
              require-online
              :selected-id="project?.deviceId ?? 'cloud'"
              @select="selectMain"
              @connect="connect"
            />
          </template>
        </MenuItem>
        <MenuGroup v-if="conversation.attachedHosts.length" label="Attached hosts">
          <MenuItem
            v-for="host in conversation.attachedHosts"
            :key="host.deviceId"
            :label="host.name"
            :indicator="isOnline(host.deviceId) ? 'success' : 'muted'"
            :indicator-label="isOnline(host.deviceId) ? 'Online' : 'Offline'"
            has-submenu
          >
            <template #submenu>
              <Menu class="w-64">
                <MenuItem
                  label="Use as main environment…"
                  :icon="Monitor"
                  :disabled="
                    mainLocked ||
                    (host.deviceId !== 'cloud' &&
                      !resources.devices.find((device) => device.id === host.deviceId)?.online)
                  "
                  @select="switchMain(host.deviceId, host.cwd)"
                />

                <MenuItem
                  label="Detach"
                  :icon="Unlink"
                  :disabled="conversation.archived"
                  @select="detach(host.deviceId)"
                />
              </Menu>
            </template>
          </MenuItem>
        </MenuGroup>
        <MenuDivider />
        <MenuItem label="Attach device…" :icon="Plus" has-submenu :disabled="conversation.archived">
          <template #submenu>
            <HostPicker
              :devices="resources.devices"
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
