<script setup lang="ts">
import { computed } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'

import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'

import {
  fileMatchesAcceptedExtensions,
  filePreviewUrl,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { useConversations } from './store'
import { useResources } from '../prototype/resources'
import type { Conversation } from '../prototype/types'

const props = defineProps<{ conversation: Conversation }>()
const store = useConversations()
const resources = useResources()

const selectedModel = computed(() =>
  resources.models[props.conversation.providerId]?.find((m) => m.id === props.conversation.modelId),
)
const canSend = computed(
  () =>
    resources.providers.some((p) => p.id === props.conversation.providerId && p.isAvailable) &&
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
        selectedModel.value?.acceptedExtensions ?? [],
      )
        ? 'message'
        : 'workspace',
    })
  }
}

function removeFile(id: string) {
  const file = props.conversation.files.find((f) => f.id === id)
  if (file?.src) URL.revokeObjectURL(file.src)
  props.conversation.files = props.conversation.files.filter((f) => f.id !== id)
}

function openProviders() {
  resources.settingsTab = 'Providers'
  resources.settingsOpen = true
}
function selectModel(providerId: string, modelId: string) {
  props.conversation.providerId = providerId
  props.conversation.modelId = modelId
  props.conversation.serviceTierId = null
}
function send() {
  if (canSend.value) store.send(props.conversation)
}

const attachments = computed(() =>
  props.conversation.files.map((file) => ({
    ...file,
    caption: file.destination === 'message' ? 'Message attachment' : 'Workspace file',
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
      :providers="resources.providers"
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
      @submit="send"
      @add-files="addFiles"
      @remove-attachment="(index) => removeFile(conversation.files[index]!.id)"
      @select-model="selectModel"
      @change-thinking="conversation.thinking = $event"
      @change-service-tier="conversation.serviceTierId = $event"
      @stop="store.stop(conversation)"
      @compact="store.compact(conversation)"
    />
  </div>
</template>
