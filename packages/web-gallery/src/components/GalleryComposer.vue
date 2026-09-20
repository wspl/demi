<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'
import type { ThinkingConfig, TokenUsage, UserContentBlock } from '@demicodes/core'
import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'
import { joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import {
  applyAttachmentUpdate,
  arrangeCapsuleFiles,
  AttachmentUploadQueue,
  attachmentFileError,
  attachTextSnippet,
  composerAttachment,
  composerAttachmentFromFile,
  composerFileNames,
  composerRemoteAttachment,
  encodeRemoteReference,
  isComposerFile,
  remoteAttachmentError,
  type AttachmentUploadUpdate,
  type ComposerAttachment,
  type ComposerAttachmentInput,
} from '@demicodes/web-ui/agent/message-input/attachments'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'
import { demoUsage } from '../fixtures/blocks'
import { demoModels, demoProviders } from '../fixtures/catalog'
import { createGalleryRemoteFileHosts } from '../fixtures/files'
import { sweepUpload } from '../fixtures/upload-sweep'

const props = withDefaults(
  defineProps<{
    placeholder: string
    running?: boolean
    compacting?: boolean
    disabled?: boolean
    /** The model catalog: `failed` tells it beside the model chip with Retry, as the product does. */
    modelLoad?: 'loading' | 'ready' | 'failed'
    /** The draft's Markdown, an attachment mark where each capsule stands; files without one follow the text. */
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
    draft: '',
    attachments: () => [],
    selectedProviderId: 'anthropic',
    selectedModelId: 'claude-sonnet',
    canConfigure: true,
  },
)
const emit = defineEmits<{
  send: [content: UserContentBlock[]]
  queue: [content: UserContentBlock[]]
  stop: []
  compact: []
  configure: []
  restore: []
  retryModels: []
  'update:messageEdit': [state: MessageEditState | null]
  submitEdit: []
}>()
const draft = ref(props.draft)
const attached = ref(props.attachments.map((item) => composerAttachment(item)))
/** Files whose capsule was deleted: they wait here for an undo. */
const aside = ref<ComposerAttachment[]>([])
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

/** Pictures of sent files stay loadable while the transcript shows them; they go with the page. */
const sentPictures: string[] = []

function forget(id: string, release: boolean) {
  uploads.cancel(id)
  const item = attached.value.find((file) => file.id === id)
  if (item && isComposerFile(item) && item.src?.startsWith('blob:')) {
    if (release) {
      URL.revokeObjectURL(item.src)
    } else {
      sentPictures.push(item.src)
    }
  }
  attached.value = attached.value.filter((file) => file.id !== id)
}

/**
 * The capsules in the text, in their order. A file whose capsule was deleted
 * waits aside for an undo to bring both back, as the product's does.
 */
function arrange(ids: string[]) {
  const next = arrangeCapsuleFiles(attached.value, aside.value, ids)
  for (const item of next.detached) {
    uploads.cancel(item.id)
  }
  attached.value = next.files
  aside.value = next.aside
  for (const item of next.restored) {
    if (isComposerFile(item) && item.phase !== 'ready') {
      upload(item.id)
    }
  }
}

/** Lets the files set aside go: they are in no message, and no undo can reach them now. */
function releaseAside() {
  for (const item of aside.value) {
    uploads.cancel(item.id)
    if (isComposerFile(item) && item.src?.startsWith('blob:')) {
      URL.revokeObjectURL(item.src)
    }
  }
  aside.value = []
}

/** A sent file as the transcript records it: a record at a made-up Host path, a picture before it, or a device's file. */
function sentBlocks(item: ComposerAttachment): UserContentBlock[] {
  if (item.kind === 'reference') {
    return [{ type: 'reference', reference: encodeRemoteReference(item.host, item.path) }]
  }
  const record: UserContentBlock = {
    type: 'attachment',
    name: item.name,
    path: `/home/demi/.demi/attachments/gallery/${item.name}`,
    mediaType: item.src ? 'image/png' : 'application/octet-stream',
    sizeBytes: 0,
    sha256: 'gallery',
    snippet: item.snippet,
  }
  return item.src ? [{ type: 'image', source: { type: 'url', url: item.src } }, record] : [record]
}

function submit() {
  const content = joinMessageContent(draft.value, attached.value.map(sentBlocks))
  if (!content.length) {
    return
  }
  draft.value = ''
  while (attached.value.length) {
    forget(attached.value[0]!.id, false)
  }
  releaseAside()
  if (props.running) {
    emit('queue', content)
  } else {
    emit('send', content)
  }
}

function upload(id: string) {
  // The sweep never fails; were it to, the file would keep its capsule with Retry, as the product's does.
  uploads.start(id, sweepUpload, (update) => applyUpdate(id, update)).catch(() => applyUpdate(id, { phase: 'failed' }))
}

function addFiles(files: File[]) {
  for (const file of files) {
    if (attachmentFileError(file, composerFileNames(attached.value))) {
      continue
    }
    const item = composerAttachmentFromFile(file)
    attached.value.push(item)
    // The snippet lands on the reactive item, so the capsule shows it.
    const held = attached.value.find((each) => each.id === item.id)
    if (held && isComposerFile(held)) {
      void attachTextSnippet(held, file)
    }
    upload(item.id)
  }
}

function retry(id: string) {
  const item = attached.value.find((file) => file.id === id)
  if (item && isComposerFile(item) && item.phase === 'failed') {
    upload(id)
  }
}

const remoteHosts = createGalleryRemoteFileHosts()
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
  releaseAside()
  while (attached.value.length) {
    forget(attached.value[0]!.id, true)
  }
  for (const url of sentPictures) {
    URL.revokeObjectURL(url)
  }
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
    @retry-models="emit('retryModels')"
    @add-files="addFiles"
    @attach-remote="remotePicker?.open()"
    @arrange-attachments="arrange"
    @retry-attachment="retry"
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
