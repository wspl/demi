<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import type { ThinkingConfig, TokenUsage } from '@demicodes/core'
import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import {
  applyAttachmentUpdate,
  AttachmentUploadQueue,
  attachmentFileError,
  attachTextSnippet,
  composerAttachment,
  composerAttachmentFromFile,
  composerFileNames,
  composerRemoteAttachment,
  isComposerFile,
  remoteAttachmentError,
  type AttachmentUploadUpdate,
  type ComposerAttachmentInput,
} from '@demicodes/web-ui/agent/message-input/attachments'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'
import { demoUsage } from '../fixtures/blocks'
import { demoModels, demoProviders } from '../fixtures/catalog'
import { createGalleryFileHosts } from '../fixtures/files'

const props = withDefaults(
  defineProps<{
    placeholder: string
    conversationId?: string
    running?: boolean
    compacting?: boolean
    draft?: string
    attachments?: ComposerAttachmentInput[]
    focused?: boolean
    attachOpen?: boolean
    dropping?: boolean
    selectedProviderId?: string
    selectedModelId?: string
    serviceTierId?: string | null
    usage?: TokenUsage
    providers?: ProviderInfo[]
    models?: Record<string, ModelInfo[]>
    canConfigure?: boolean
    archived?: boolean
    messageEdit?: MessageEditState | null
  }>(),
  {
    conversationId: 'demo',
    draft: '',
    attachments: () => [],
    selectedProviderId: 'anthropic',
    selectedModelId: 'claude-sonnet',
    canConfigure: true,
  },
)
const emit = defineEmits<{
  send: [text: string]
  queue: [text: string]
  stop: []
  compact: []
  configure: []
  restore: []
  'update:messageEdit': [state: MessageEditState | null]
  submitEdit: []
}>()
const draft = ref(props.draft)
const attached = ref(props.attachments.map((item) => composerAttachment(item)))
const uploads = new AttachmentUploadQueue()
const providerId = ref(props.selectedProviderId)
const modelId = ref(props.selectedModelId)
const tier = ref(props.serviceTierId ?? null)
const thinking = ref<ThinkingConfig>({
  type: 'effort',
  effort: 'medium',
  summary: null,
})

function applyUpdate(id: string, update: AttachmentUploadUpdate) {
  const item = attached.value.find((file) => file.id === id)
  if (item) {
    applyAttachmentUpdate(item, update)
  }
}

function remove(id: string) {
  uploads.cancel(id)
  const item = attached.value.find((file) => file.id === id)
  if (item && isComposerFile(item) && item.src?.startsWith('blob:')) {
    URL.revokeObjectURL(item.src)
  }
  attached.value = attached.value.filter((file) => file.id !== id)
}

function submit() {
  const text =
    draft.value.trim() || attached.value.map((item) => item.name).join(', ')
  if (!text) {
    return
  }
  draft.value = ''
  while (attached.value.length) {
    remove(attached.value[0]!.id)
  }
  if (props.running) {
    emit('queue', text)
  } else {
    emit('send', text)
  }
}

function addFiles(files: File[]) {
  for (const file of files) {
    if (attachmentFileError(file, composerFileNames(attached.value))) {
      continue
    }
    const item = composerAttachmentFromFile(file)
    attached.value.push(item)
    void attachTextSnippet(attached.value.find((held) => held.id === item.id) as typeof item, file)
    uploads.startPrototype(item.id, (update) => applyUpdate(item.id, update), remove)
  }
}

const remoteHosts = createGalleryFileHosts()
const remotePicker = ref<InstanceType<typeof RemoteFilePicker>>()
function attachRemote(file: { host: string; path: string }) {
  const error = remoteAttachmentError(file.path, file.host, attached.value)
  if (!error) {
    attached.value.push(composerRemoteAttachment(file))
  }
}
function selectModel(provider: string, model: string) {
  providerId.value = provider
  modelId.value = model
}
onBeforeUnmount(() => {
  uploads.cancelAll()
  while (attached.value.length) {
    remove(attached.value[0]!.id)
  }
})

defineExpose({
  setDraft(text: string) {
    draft.value = text
  },
})
</script>

<template>
  <SessionComposer
    v-bind="props"
    v-model:draft="draft"
    @update:message-edit="emit('update:messageEdit', $event)"
    @submit-edit="emit('submitEdit')"
    :attachments="attached"
    :providers="props.providers ?? demoProviders"
    :models="props.models ?? demoModels"
    :can-configure="canConfigure !== false"
    :archived="archived"
    :selected-provider-id="providerId"
    :selected-model-id="modelId"
    :service-tier-id="tier"
    :thinking-config="thinking"
    :usage="props.usage ?? demoUsage"
    remote-files
    @submit="submit"
    @configure="emit('configure')"
    @restore="emit('restore')"
    @add-files="addFiles"
    @attach-remote="remotePicker?.open()"
    @remove-attachment="remove"
    @select-model="selectModel"
    @change-thinking="thinking = $event"
    @change-service-tier="tier = $event"
    @stop="emit('stop')"
    @compact="emit('compact')"
  />
  <RemoteFilePicker
    ref="remotePicker"
    :overlay-store="appOverlayStore"
    :hosts="remoteHosts"
    @select="attachRemote"
  />
</template>
