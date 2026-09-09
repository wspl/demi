<script setup lang="ts">
import { computed, ref } from 'vue'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'

import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'

import {
  composerRemoteAttachment,
  remoteAttachmentError,
} from '@demicodes/web-ui/agent/message-input/attachments'
import type { ThinkingConfig } from '@demicodes/core'
import { executionFor } from '../targets/execution'
import { composerModel } from '@demicodes/web-ui/agent/model-selection'
import { useConversations } from './store'
import { useResources } from '../state/resources'
import { fileSourceFor, placesFor } from '../devices/files'
import type { Conversation } from '../state/types'

const props = defineProps<{ conversation: Conversation }>()
const store = useConversations()
const resources = useResources()

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

function addFiles(files: File[]) {
  store.addFiles(
    props.conversation,
    files,
    selectedModel.value?.acceptedExtensions ?? null,
  )
}

function removeFile(id: string) {
  store.removeFile(props.conversation, id)
}

const thinking = computed<ThinkingConfig | undefined>(() => {
  const effort = props.conversation.model.thinkingEffort
  if (effort === null) {
    return undefined
  }
  if (effort === 'disabled') {
    return { type: 'disabled' }
  }
  return {
    type: 'effort',
    effort,
    summary: null,
  }
})
const usage = computed(
  () =>
    props.conversation.blocks.findLast((block) => block.type === 'response')
      ?.usage ?? null,
)
function openProviders() {
  resources.settingsTab = 'models'
  resources.settingsOpen = true
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
const remoteHosts = computed(() => {
  const main = execution.value
  const hosts = [
    ...(main.kind === 'device' && main.deviceId
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
        resources.devices.some((device) => device.id === host.deviceId),
      )
      .map((host) => ({
        id: host.deviceId,
        label: host.name,
        online: host.online,
      })),
  ]
  return hosts.map((host) => {
    const device = resources.devices.find((device) => device.id === host.id) ?? null
    const cwd =
      host.id === main.deviceId
        ? main.path
        : props.conversation.attachedHosts.find(
            (attached) => attached.deviceId === host.id,
          )?.cwd
    return {
      ...host,
      source: fileSourceFor(device),
      places: placesFor(device, resources.projects),
      cwd: cwd ?? undefined,
    }
  })
})
const remotePicker = ref<InstanceType<typeof RemoteFilePicker>>()
function attachRemote(file: { deviceId: string; host: string; path: string }) {
  const error = remoteAttachmentError(file.path, file.host, props.conversation.files)
  if (error) {
    store.notice = error
    return
  }
  props.conversation.files.push({
    ...composerRemoteAttachment(file),
    deviceId: file.deviceId,
  })
}
</script>

<template>
  <div>
    <SessionComposer
      v-model:draft="conversation.draft"
      placeholder="Ask Demi…"
      :conversation-id="conversation.id"
      :running="conversation.phase === 'running'"
      :compacting="conversation.phase === 'compacting'"
      :disabled="conversation.submission === 'sending' || !!conversation.pendingSend"
      :can-configure="resources.canConfigure"
      :attachments="conversation.files.filter((file) => !conversation.pendingSend?.fileIds.includes(file.id))"
      :providers="resources.providerInfos"
      :models="resources.models"
      :selected-provider-id="conversation.model.providerId"
      :selected-model-id="conversation.model.modelId"
      :thinking-config="thinking"
      :service-tier-id="conversation.model.serviceTierId"
      :usage="usage"
      :remote-files="remoteHosts.length > 0"
      :archived="conversation.archived"
      @submit="send"
      @configure="openProviders"
      @restore="store.archive([conversation.id], false)"
      @add-files="addFiles"
      @attach-remote="remotePicker?.open()"
      @remove-attachment="removeFile"
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
