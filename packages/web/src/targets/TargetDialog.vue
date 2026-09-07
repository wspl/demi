<script setup lang="ts">
import { computed, ref } from 'vue'
import { X, Plus, FolderOpen } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import FileBrowser from '@demicodes/web-ui/files/FileBrowser.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'

import { useConversations } from '../conversation/store'
import { useResources } from '../prototype/resources'
import { browserHosts, fileSourceFor, placesFor } from '../prototype/files'

const props = defineProps<{ conversationId: string | null }>()
const resources = useResources()
const conversations = useConversations()

const name = ref('')
const path = ref('/Users/zan/Projects/')
const deviceId = ref(resources.devices[0]?.id ?? 'cloud')
const showCreate = ref(resources.targetMode === 'create')
const current = computed(() => conversations.items.find((c) => c.id === props.conversationId))
const message = ref('')
const browsing = ref(false)
const browsingDevice = computed(() => resources.devices.find((d) => d.id === deviceId.value) ?? null)
const browserSource = computed(() => fileSourceFor(browsingDevice.value))
const browserPlaces = computed(() => placesFor(browsingDevice.value, resources.projects))
const browserHostList = computed(() => browserHosts(resources.devices, false))

/** The browser's folder becomes the project's directory, and its name when none is typed. */
function pickDirectory(chosen: string) {
  path.value = chosen
  if (!name.value.trim()) name.value = chosen.split('/').filter(Boolean).at(-1) ?? ''
  browsing.value = false
}

function close() {
  resources.targetOpen = false
}

function select(id: string | null) {
  if (!current.value) return
  conversations.move([current.value.id], id)
  close()
}

function create() {
  if (!name.value.trim()) return
  const cloud = deviceId.value === 'cloud'
  const device = resources.devices.find((d) => d.id === deviceId.value)
  if (!cloud && (!device || !path.value.startsWith('/'))) {
    message.value = 'Select a device and enter an absolute directory path.'
    return
  }
  const id = crypto.randomUUID()
  resources.projects.push({
    id,
    name: name.value.trim(),
    deviceId: deviceId.value,
    host: cloud ? 'Cloud' : device!.name,
    hostKind: cloud ? 'cloud' : 'device',
    path: cloud ? '/home/demi' : path.value.trim(),
    branch: null,
  })
  if (resources.targetMode === 'switch') select(id)
  else close()
}
</script>

<template>
  <!-- One dialog: the browser is a page of it, since a dialog over a dialog closes the first. -->
  <Dialog
    :is-open="true"
    :overlay-store="appOverlayStore"
    :size="browsing ? 'lg' : 'md'"
    label="Working environment"
    @close="close"
  >
    <div v-if="browsing" class="flex h-[32rem] min-h-0 flex-col">
      <header class="flex h-11 shrink-0 select-none items-center border-b border-line px-4 pr-12">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Select folder</h3>
      </header>
      <FileBrowser
        mode="directory"
        :source="browserSource"
        :initial-path="path.startsWith('/') ? path.replace(/\/$/, '') : undefined"
        :places="browserPlaces"
        :hosts="browserHostList"
        :host-id="deviceId"
        @select="pickDirectory"
        @cancel="browsing = false"
        @update:host-id="deviceId = $event"
      />
    </div>
    <header v-else class="flex select-none items-center justify-between border-b border-line px-4 py-3">
      <h2 class="text-[15px] font-medium text-fg-emphasis">
        {{ showCreate ? 'New project' : 'Working environment' }}
      </h2>
      <IconButton :icon="X" variant="ghost" aria-label="Close environment picker" @click="close" />
    </header>
    <div v-if="!browsing" class="settings-content p-4">
      <template v-if="!showCreate">
        <Menu class="mb-3 w-full" iconless>
          <MenuItem
            label="No project"
            choice
            :is-selected="!current?.projectId"
            :disabled="!!current?.stream"
            @select="select(null)"
          />
          <MenuItem
            v-for="project in resources.projects"
            :key="project.id"
            :label="project.name"
            :value="project.host"
            :title="project.path"
            choice
            :is-selected="current?.projectId === project.id"
            :disabled="!!current?.stream"
            @select="select(project.id)"
          ></MenuItem>
        </Menu>
        <Button :disabled="!!current?.stream" @click="showCreate = true">
          <Plus :size="14" />
          New project
        </Button>
      </template>
      <form v-else class="space-y-4" @submit.prevent="create">
        <label>
          Project name
          <TextInput v-model="name" focused required maxlength="64" placeholder="My next idea" />
        </label>
        <label>Device</label>
        <Dropdown :overlay-store="appOverlayStore" variant="default">
          <template #trigger>
            {{
              deviceId === 'cloud'
                ? 'Cloud'
                : resources.devices.find((d) => d.id === deviceId)?.name
            }}
          </template>
          <template #content>
            <Menu>
              <MenuItem
                v-for="device in resources.devices"
                :key="device.id"
                :label="device.name"
                choice
                :is-selected="deviceId === device.id"
                @select="deviceId = device.id"
              />
              <MenuItem
                label="Cloud"
                choice
                :is-selected="deviceId === 'cloud'"
                @select="deviceId = 'cloud'"
              />
            </Menu>
          </template>
        </Dropdown>
        <label v-if="deviceId !== 'cloud'">
          Directory
          <div class="flex items-center gap-2">
            <TextInput v-model="path" required placeholder="/path/to/project" />
            <Button class="shrink-0" :disabled="!browsingDevice?.online" @click="browsing = true">
              <FolderOpen :size="14" />
              Browse…
            </Button>
          </div>
        </label>
        <p v-if="message" class="hint" role="alert">{{ message }}</p>
        <Button variant="primary" :disabled="!name.trim()" @click="create">Create project</Button>
      </form>
    </div>
  </Dialog>
</template>
