<script setup lang="ts">
import { computed, ref } from 'vue'
import { Cloud, Link, Monitor, Pencil, Plus, Unlink, X } from '@lucide/vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { Conversation, Project } from '../prototype/types'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'

const props = defineProps<{ conversation: Conversation; project?: Project }>()
const emit = defineEmits<{ switchMain: [deviceId: string, cwd?: string] }>()
const resources = useResources()
const store = useConversations()
const open = ref(false)
const renaming = ref<string | null>(null)
const name = ref('')
const error = ref('')
const mainLocked = computed(() => !!props.conversation.stream || props.conversation.archived)
const available = computed(() =>
  resources.devices
    .filter(
      (device) =>
        device.id !== props.project?.deviceId &&
        !props.conversation.attachedHosts.some((host) => host.deviceId === device.id),
    )
    .map((device) => ({
      id: device.id,
      label: `${device.name} · ${device.online ? 'Online' : 'Offline'}`,
      icon: Monitor,
    })),
)
function switchMain(deviceId: string, cwd?: string) {
  if (mainLocked.value) return
  open.value = false
  emit('switchMain', deviceId, cwd)
}
function attach(id: string) {
  store.attachHost(props.conversation, id)
  open.value = false
}
function rename(deviceId: string, alias: string) {
  open.value = false
  name.value = alias
  error.value = ''
  renaming.value = deviceId
}
function saveName() {
  if (!renaming.value) return
  if (store.renameHost(props.conversation, renaming.value, name.value)) renaming.value = null
  else error.value = 'Enter a nonempty name unique within this conversation.'
}
function detach(id: string) {
  store.detachHost(props.conversation, id)
  open.value = false
}
function hostless() {
  if (mainLocked.value) return
  store.move([props.conversation.id], null)
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
          :is="!project ? Link : project.hostKind === 'cloud' ? Cloud : Monitor"
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
        <MenuGroup label="Main execution environment">
          <div class="px-3 py-2 text-chrome">
            <p class="flex items-center justify-between gap-2 text-fg">
              <span>{{ project?.host ?? 'Hostless' }}</span>
              <span v-if="project" class="text-[11px] text-fg-subtle">
                {{
                  project.hostKind === 'cloud' ||
                  resources.devices.find((device) => device.id === project.deviceId)?.online
                    ? 'Online'
                    : 'Offline'
                }}
              </span>
            </p>
            <p class="truncate text-[11px] text-fg-subtle" :title="project?.path">
              {{ project?.path ?? 'No main device' }}
            </p>
          </div>
          <MenuItem
            :icon="Monitor"
            label="Switch main environment…"
            :disabled="mainLocked"
            has-submenu
          >
            <template #submenu>
              <Menu class="w-60">
                <MenuItem
                  v-for="device in resources.devices"
                  :key="device.id"
                  :icon="Monitor"
                  :label="device.name"
                  :disabled="!device.online || device.id === project?.deviceId"
                  :disabled-reason="!device.online ? 'Device offline' : undefined"
                  @select="
                    switchMain(
                      device.id,
                      conversation.attachedHosts.find((host) => host.deviceId === device.id)?.cwd,
                    )
                  "
                />
                <MenuItem
                  :icon="Cloud"
                  label="Cloud"
                  :disabled="project?.hostKind === 'cloud'"
                  @select="switchMain('cloud')"
                />
                <MenuItem :icon="Unlink" label="Hostless" :disabled="!project" @select="hostless" />
              </Menu>
            </template>
          </MenuItem>
          <p class="px-3 py-1 text-[11px] text-fg-subtle">
            When switched, the previous main host stays attached.
          </p>
        </MenuGroup>
        <MenuGroup :label="`Attached hosts · ${conversation.attachedHosts.length}`">
          <p v-if="!conversation.attachedHosts.length" class="px-3 py-2 text-[12px] text-fg-subtle">
            No additional devices attached.
          </p>
          <MenuItem
            v-for="host in conversation.attachedHosts"
            :key="host.deviceId"
            :label="`${host.name} · ${host.deviceId === 'cloud' || resources.devices.find((device) => device.id === host.deviceId)?.online ? 'Online' : 'Offline'}`"
            :icon="host.deviceId === 'cloud' ? Cloud : Monitor"
            has-submenu
          >
            <template #submenu>
              <Menu class="w-64">
                <div class="px-3 py-2 text-[11px] text-fg-subtle">
                  <p>
                    {{
                      resources.devices.find((device) => device.id === host.deviceId)?.name ??
                      'Cloud'
                    }}
                  </p>
                  <p class="break-all select-text">{{ host.cwd }}</p>
                </div>
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
                  label="Rename…"
                  :icon="Pencil"
                  :disabled="conversation.archived"
                  @select="rename(host.deviceId, host.name)"
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
            <Menu
              :items="available"
              filterable
              filter-placeholder="Search devices…"
              empty-text="No more devices to attach"
              class="w-64"
              @select="attach"
            />
          </template>
        </MenuItem>
        <MenuItem label="Connect new device…" :icon="Link" @select="connect" />
        <p v-if="conversation.stream" class="px-3 py-2 text-[11px] text-fg-subtle">
          Main environment can change after this turn finishes.
        </p>
      </Menu>
    </template>
  </Dropdown>
  <Dialog
    :is-open="!!renaming"
    :overlay-store="appOverlayStore"
    label="Rename attached host"
    @close="renaming = null"
  >
    <form class="space-y-4 p-4" @submit.prevent="saveName">
      <div class="flex items-center justify-between">
        <h2 class="text-chrome font-medium">Rename attached host</h2>
        <IconButton :icon="X" variant="ghost" aria-label="Close rename" @click="renaming = null" />
      </div>
      <label class="block text-chrome">
        Conversation name
        <TextInput v-model="name" required aria-label="Attached host name" />
      </label>
      <p class="text-[12px] text-fg-subtle">
        The agent uses this name to address the host in this conversation.
      </p>
      <p v-if="error" class="text-chrome text-on-warning">{{ error }}</p>
      <Button @click="saveName">Save name</Button>
    </form>
  </Dialog>
</template>
