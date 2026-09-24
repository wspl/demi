<script setup lang="ts">
import { computed, ref } from 'vue'
import { Cloud } from '@lucide/vue'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { reportError } from '@demicodes/web-ui/infra/errors'

import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'

import {
  composerRemoteAttachment,
  remoteAttachmentError,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { composerCapsule } from '@demicodes/web-ui/agent/message-editor/capsules'
import { executionFor } from '../targets/execution'
import { composerModel, intentThinkingConfig } from '@demicodes/web-ui/agent/model-selection'
import { useConversations } from './store'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { placesFor } from '../devices/files'
import { fileSource } from '../api/files'
import { uploadAttachment } from '../api/uploads'
import type { Conversation } from '../state/types'

const props = defineProps<{ conversation: Conversation }>()
const store = useConversations()
const resources = useResources()
const product = useProduct()

const modelState = computed(() =>
  composerModel(
    resources.providerInfos,
    resources.models,
    props.conversation.model.providerId,
    props.conversation.model.modelId,
  ),
)
const selectedModel = computed(() => modelState.value.selected?.model)
const canSend = computed(() => modelState.value.kind === 'ready')

const composer = ref<InstanceType<typeof SessionComposer>>()

/** The store takes the files; their capsules go into the message where the composer said they would land. */
function addFiles(files: File[]) {
  composer.value?.insertCapsules(store.addFiles(props.conversation, files).map(composerCapsule))
}

const thinking = computed(() => intentThinkingConfig(props.conversation.model))
const usage = computed(
  () =>
    props.conversation.blocks.findLast((block) => block.type === 'response')
      ?.usage ?? null,
)
function openProviders() {
  resources.openSettings('models')
}
function selectModel(providerId: string, modelId: string) {
  void store.selectModel(props.conversation, providerId, modelId)
}
function send() {
  if (canSend.value) {
    void store.send(props.conversation)
  }
}

const execution = computed(() => executionFor(props.conversation))
// A reset holds the conversations that cannot work without the Cloud: it is
// their target, or where their provider's process runs. The account snapshot
// the page polls says when it ends, and the input returns by itself.
const usesCloud = computed(() =>
  execution.value.kind === 'cloud' ||
  product.catalog.find((provider) => provider.providerId === props.conversation.model.providerId)
    ?.requiresProcessCapableHost === true,
)
const hold = computed(() =>
  usesCloud.value && product.snapshot?.cloud?.state === 'resetting'
    ? 'Cloud is resetting.'
    : null,
)
const remoteHosts = computed(() => {
  const main = execution.value
  const hosts = [
    ...(main.deviceId
      ? [
          {
            id: main.deviceId,
            label: main.name,
            online: main.online,
          },
        ]
      : []),
    ...props.conversation.attachedHosts
      .filter((host) =>
        resources.deviceById(host.deviceId) !== null,
      )
      .map((host) => ({
        id: host.deviceId,
        label: host.name,
        online: resources.deviceById(host.deviceId)?.online ?? false,
      })),
  ]
  return hosts.map((host) => {
    const device = resources.deviceById(host.id)
    const cwd =
      host.id === main.deviceId
        ? main.path
        : props.conversation.attachedHosts.find(
            (attached) => attached.deviceId === host.id,
          )?.cwd
    return {
      ...host,
      canWake: device?.kind === 'managed',
      icon: device?.kind === 'managed' ? Cloud : undefined,
      source: fileSource({
        directory: `/conversations/${encodeURIComponent(props.conversation.id)}/hosts/${encodeURIComponent(host.id)}/fs`,
      }, device),
      places: placesFor(device, resources.projects),
      cwd: cwd ?? undefined,
    }
  })
})
const remotePicker = ref<InstanceType<typeof RemoteFilePicker>>()
function attachRemote(file: { deviceId: string; host: string; path: string }) {
  const error = remoteAttachmentError(
    file.path,
    file.host,
    props.conversation.files,
  )
  if (error) {
    reportError("Couldn't attach", error, { userVisible: true, expected: true })
    return
  }
  const item = { ...composerRemoteAttachment(file), deviceId: file.deviceId }
  props.conversation.files.push(item)
  composer.value?.insertCapsules([composerCapsule(item)])
}
</script>

<template>
  <div>
    <SessionComposer
      ref="composer"
      :model-load="product.catalogLoad"
      @retry-models="product.revalidate"
      v-model:draft="conversation.draft"
      v-model:message-edit="conversation.messageEdit"
      :upload="uploadAttachment"
      @submit-edit="store.submitEdit(conversation)"
      placeholder="Ask Demi…"
      :running="conversation.phase === 'running'"
      :compacting="conversation.phase === 'compacting'"
      :disabled="
        conversation.submission === 'sending' || !!conversation.pendingSend
      "
      :can-configure="resources.canConfigure"
      :attachments="
        conversation.files.filter(
          (file) => !conversation.pendingSend?.fileIds.includes(file.id),
        )
      "
      :providers="resources.providerInfos"
      :models="resources.models"
      :selected-provider-id="conversation.model.providerId"
      :selected-model-id="conversation.model.modelId"
      :thinking-config="thinking"
      :service-tier-id="conversation.model.serviceTierId"
      :usage="usage"
      :remote-files="remoteHosts.length > 0"
      :archived="conversation.archived"
      :hold="hold"
      @submit="send"
      @configure="openProviders"
      @restore="store.archive([conversation.id], false)"
      @add-files="addFiles"
      @attach-remote="remotePicker?.open()"
      @arrange-attachments="store.arrangeFiles(conversation, $event)"
      @retry-attachment="store.retryFile(conversation, $event)"
      @select-model="selectModel"
      @change-thinking="store.setThinking(conversation, $event)"
      @change-service-tier="store.setTier(conversation, $event)"
      @stop="store.stop(conversation)"
      @compact="store.compact(conversation)"
    />
    <RemoteFilePicker
      ref="remotePicker"
      :overlay-store="appOverlayStore"
      :hosts="remoteHosts"
      @select="attachRemote"
    />
  </div>
</template>
