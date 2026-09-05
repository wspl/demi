<script setup lang="ts">
import { computed, ref } from 'vue'
import { X, Plus } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'

import { useConversations } from '../conversation/store'
import { useResources } from '../prototype/resources'

const props = defineProps<{ conversationId: string | null }>()
const resources = useResources()
const conversations = useConversations()

const name = ref('')
const path = ref('/Users/zan/Projects/')
const deviceId = ref(resources.devices[0]?.id ?? 'cloud')
const showCreate = ref(resources.targetMode === 'create')
const current = computed(() => conversations.items.find((c) => c.id === props.conversationId))
const mainDevice = computed(
  () => resources.projects.find((p) => p.id === current.value?.projectId)?.deviceId,
)
const message = ref('')

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
    path: cloud ? '/home/demi' : path.value.trim(),
    branch: null,
    branches: [],
  })
  if (resources.targetMode === 'switch') select(id)
  else close()
}

function attach(id: string, checked: boolean) {
  const c = current.value
  if (!c) return
  c.attachedHosts = checked
    ? [...c.attachedHosts, id]
    : c.attachedHosts.filter((host) => host !== id)
}
</script>

<template>
  <Dialog
    :is-open="true"
    :overlay-store="appOverlayStore"
    label="Working environment"
    @close="close"
  >
    <header class="flex select-none items-center justify-between border-b border-line px-4 py-3">
      <h2 class="text-[15px] font-medium text-fg-emphasis">
        {{ showCreate ? 'New project' : 'Working environment' }}
      </h2>
      <IconButton :icon="X" variant="ghost" aria-label="Close environment picker" @click="close" />
    </header>
    <div class="settings-content p-4">
      <template v-if="!showCreate">
        <p class="hint">
          A project sets the machine and directory. Existing files stay where they are.
        </p>
        <p v-if="current?.stream" class="hint">
          Finish the running turn before switching environments.
        </p>
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
            choice
            :is-selected="current?.projectId === project.id"
            :disabled="!!current?.stream"
            @select="select(project.id)"
          >
            <span class="flex min-w-0 flex-1 flex-col py-1">
              <span>{{ project.name }}</span>
              <span class="truncate text-[11px] text-fg-subtle">
                {{ project.host }} · {{ project.path }}
              </span>
            </span>
          </MenuItem>
        </Menu>
        <Button :disabled="!!current?.stream" @click="showCreate = true">
          <Plus :size="14" />
          New project
        </Button>
        <template v-if="current">
          <h3 class="mt-5">Attached hosts</h3>
          <p class="hint">Additional devices this conversation can work with.</p>
          <div
            v-for="device in resources.devices.filter((d) => d.id !== mainDevice)"
            :key="device.id"
            class="py-1"
          >
            <Checkbox
              :model-value="current.attachedHosts.includes(device.id)"
              :label="`${device.name} · ${device.online ? 'Online' : 'Offline'}`"
              @update:model-value="attach(device.id, $event)"
            />
          </div>
        </template>
      </template>
      <form v-else class="space-y-4" @submit.prevent="create">
        <p class="hint">Keep related conversations around a working directory.</p>
        <label>
          Project name
          <TextInput v-model="name" focused required maxlength="64" placeholder="My next idea" />
        </label>
        <label>Device</label>
        <Dropdown :overlay-store="appOverlayStore" variant="default">
          <template #trigger>
            {{
              deviceId === 'cloud'
                ? 'Cloud · simulated'
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
                label="Cloud · simulated"
                choice
                :is-selected="deviceId === 'cloud'"
                @select="deviceId = 'cloud'"
              />
            </Menu>
          </template>
        </Dropdown>
        <label v-if="deviceId !== 'cloud'">
          Directory
          <TextInput v-model="path" required placeholder="/path/to/project" />
        </label>
        <p v-else class="hint">A simulated cloud environment will be assigned to this project.</p>
        <p v-if="message" class="hint" role="alert">{{ message }}</p>
        <Button variant="primary" :disabled="!name.trim()" @click="create">Create project</Button>
      </form>
    </div>
  </Dialog>
</template>
