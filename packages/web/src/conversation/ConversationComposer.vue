<script setup lang="ts">
import { computed, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'

import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'

import {
  fileMatchesAcceptedExtensions,
  filePreviewUrl,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { useConversations } from './store'
import { useResources } from '../prototype/resources'
import { fileSourceFor, placesFor } from '../prototype/files'
import type { Conversation } from '../prototype/types'

const props = defineProps<{ conversation: Conversation }>()
const store = useConversations()
const resources = useResources()

const selectedModel = computed(() =>
  resources.models[props.conversation.providerId]?.find(
    (m) => m.id === props.conversation.modelId
  ),
)
const canSend = computed(
  () =>
    resources.providerInfos.some(
      (p) => p.id === props.conversation.providerId && p.isAvailable
    ) &&
    !!selectedModel.value,
)

function addFiles(files: File[]) {
  for (const file of files) {
    if (!file.size || file.size > 25 * 1024 * 1024) {
      store.notice = `${file.name}: choose a nonempty file smaller than 25 MB.`
      continue
    }
    if (props.conversation.files.some((existing) => existing.name === file.name)) {
      store.notice = `${file.name} is already attached.`
      continue
    }
    props.conversation.files.push({
      id: crypto.randomUUID(),
      name: file.name,
      src: filePreviewUrl(file),
      destination: fileMatchesAcceptedExtensions(
        file,
        selectedModel.value?.acceptedExtensions ?? null,
      )
        ? 'message'
        : 'workspace',
    })
  }
}

function removeFile(id: string) {
  const file = props.conversation.files.find((f) => f.id === id)
  if (file?.src)
    URL.revokeObjectURL(file.src)
  props.conversation.files = props.conversation.files.filter((f) => f.id !== id)
}

function openProviders() {
  resources.settingsTab = 'models'
  resources.settingsOpen = true
}
function selectModel(providerId: string, modelId: string) {
  props.conversation.providerId = providerId
  props.conversation.modelId = modelId
  props.conversation.serviceTierId = null
}
function send() {
  if (canSend.value)
    store.send(props.conversation)
}

/**
 * A remote file: the conversation's main host and its attached hosts, browsed from the
 * workspace directory. The chosen path lands in the draft as a reference; the prototype
 * keeps no bytes.
 */
const project = computed(
  () =>
    resources.projects.find((item) => item.id === props.conversation.projectId)
)
const remoteHosts = computed(() => {
  const main = project.value
    ? [
      {
        id: project.value.deviceId,
        label: project.value.host,
        online: project.value.hostKind === 'cloud' ||
          !!resources.devices.find((d) => d.id === project.value!.deviceId)?.online
      }
    ]
    : []
  const attached = props.conversation.attachedHosts.map(
    (host) => ({
      id: host.deviceId,
      label: host.name,
      online: !!resources.devices.find((d) => d.id === host.deviceId)?.online
    })
  )
  return [...main, ...attached]
})
const remoteHostId = ref<string | null>(null)
const remoteDevice = computed(
  () => resources.devices.find((d) => d.id === remoteHostId.value) ?? null
)
const remoteSource = computed(() => fileSourceFor(remoteDevice.value))
const remotePlaces = computed(() => placesFor(remoteDevice.value, resources.projects))
const remoteStart = computed(() => {
  if (remoteHostId.value === project.value?.deviceId)
    return project.value?.path
  return props.conversation.attachedHosts.find(
    (host) => host.deviceId === remoteHostId.value
  )?.cwd
})
function openRemote() {
  remoteHostId.value = remoteHosts.value[0]?.id ?? null
}
function attachRemote(path: string) {
  const draft = props.conversation.draft
  props.conversation.draft = draft && !/\s$/.test(draft)
    ? `${draft} ${path}`
    : `${draft}${path}`
  remoteHostId.value = null
}

const attachments = computed(() =>
  props.conversation.files.map((file) => ({
    ...file,
    caption: file.destination === 'message'
      ? 'Message attachment'
      : 'Workspace file',
  })),
)
</script>

<template>
  <div>
    <div
      v-if="!canSend"
      class="mb-2 flex items-center justify-between gap-2 text-chrome text-fg-muted"
    >
      <span>Choose an available provider to send a message.</span>
      <Button variant="ghost" @click="openProviders">Open settings</Button>
    </div>
    <SessionComposer
      v-model:draft="conversation.draft"
      placeholder="Ask Demi…"
      :conversation-id="conversation.id"
      :running="!!conversation.stream"
      :disabled="!canSend"
      :attachments="attachments"
      :providers="resources.providerInfos"
      :models="resources.models"
      :selected-provider-id="conversation.providerId"
      :selected-model-id="conversation.modelId"
      :thinking-config="conversation.thinking"
      :service-tier-id="conversation.serviceTierId"
      :usage="{
        inputTokens: conversation.blocks.length * 120,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
      }"
      :remote-files="remoteHosts.length > 0"
      @submit="send"
      @add-files="addFiles"
      @attach-remote="openRemote"
      @remove-attachment="(index) => removeFile(conversation.files[index]!.id)"
      @select-model="selectModel"
      @change-thinking="conversation.thinking = $event"
      @change-service-tier="conversation.serviceTierId = $event"
      @stop="store.stop(conversation)"
      @compact="store.compact(conversation)"
    />
    <FileBrowserDialog
      :is-open="!!remoteHostId"
      :overlay-store="appOverlayStore"
      mode="file"
      title="Attach remote file"
      :source="remoteSource"
      :initial-path="remoteStart"
      :places="remotePlaces"
      :hosts="remoteHosts"
      :host-id="remoteHostId ?? undefined"
      confirm-label="Attach"
      @select="attachRemote"
      @close="remoteHostId = null"
      @update:host-id="remoteHostId = $event"
    />
  </div>
</template>
