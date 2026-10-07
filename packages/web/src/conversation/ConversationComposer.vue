<script setup lang="ts">
import { computed, ref } from 'vue'
import { Cloud } from '@lucide/vue'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { reportError } from '@demicodes/web-ui/infra/errors'

import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'

import {
  composerRemoteAttachment,
  fileNameFromPath,
  remoteAttachmentError,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { composerCapsule } from '@demicodes/web-ui/agent/message-editor/capsules'
import { executionFor } from '../targets/execution'
import { composerModel } from '@demicodes/web-ui/agent/model-selection'
import { useConversations } from './store'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { usePreferences } from '../state/preferences'
import { placesFor } from '../devices/files'
import { fileSource } from '../api/files'
import { uploadAttachment } from '../api/uploads'
import type { Conversation } from '../state/types'
import { cliPackageOf } from '../state/catalog'
import { useSettingsAddress } from '../settings/address'

const props = defineProps<{ conversation: Conversation }>()
const store = useConversations()
const resources = useResources()
const preferences = usePreferences()
const product = useProduct()
const settings = useSettingsAddress()

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

/** The version of the draft a later save replaced, which the composer offers to restore. */
const replaced = computed(() => {
  const version = props.conversation.savedDraft?.replaced
  return version
    ? {
        markdown: version.text,
        fileNames: version.files.map((file) =>
          file.type === 'upload' ? file.fileName : fileNameFromPath(file.path),
        ),
      }
    : null
})

function openProviders() {
  void settings.open('models')
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
  cliPackageOf(product.catalog, props.conversation.model.providerId) !== null,
)
const hold = computed(() =>
  usesCloud.value && product.snapshot?.cloud.state === 'resetting'
    ? 'Cloud is resetting.'
    : null,
)
/** The primary Host when it is a paired device whose runner is not connected. */
const offlineHost = computed(() => {
  const primary = execution.value
  if (primary.kind !== 'device' || primary.state !== 'offline') {
    return null
  }
  const start = resources.deviceById(primary.deviceId)?.start
  return start ? { name: primary.name, start } : null
})
const remoteHosts = computed(() => {
  const primary = execution.value
  const hosts = [
    ...(primary.deviceId
      ? [
          {
            id: primary.deviceId,
            label: primary.name,
            state: primary.state,
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
        state: resources.deviceById(host.deviceId)?.state ?? 'offline',
      })),
  ]
  return hosts.map((host) => {
    const device = resources.deviceById(host.id)
    const cwd =
      host.id === primary.deviceId
        ? primary.path
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
    reportError("Couldn't Attach", error, { userVisible: true, expected: true })
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
      @retry-models="product.reloadModels"
      v-model:draft="conversation.draft"
      v-model:message-edit="conversation.messageEdit"
      :upload="uploadAttachment"
      @submit-edit="store.submitEdit(conversation)"
      placeholder="Ask Demi…"
      :running="conversation.phase === 'running'"
      :compacting="conversation.phase === 'compacting'"
      :disabled="!!conversation.pendingSend"
      :can-configure="resources.canConfigure"
      :attachments="
        conversation.files.filter(
          (file) => !conversation.pendingSend?.fileIds.includes(file.id),
        )
      "
      :providers="resources.providerInfos"
      :models="resources.models"
      :model-settings="conversation.model"
      :usage="conversation.contextUsage"
      :remote-files="remoteHosts.length > 0"
      :archived="conversation.archived"
      :hold="hold"
      :offline-host="offlineHost"
      :replaced="replaced"
      :draft-shown="conversation.draftShown"
      :plugins-changed="conversation.pluginsChanged"
      :reloading="store.pendingChanges.includes(conversation.id)"
      @restore-replaced="store.restoreReplaced(conversation)"
      @dismiss-replaced="store.dismissReplaced(conversation)"
      @reload-plugins="store.reloadPlugins(conversation)"
      @submit="send"
      @configure="openProviders"
      @restore="store.archive([conversation.id], false)"
      @add-files="addFiles"
      @attach-remote="remotePicker?.open()"
      @arrange-attachments="store.arrangeFiles(conversation, $event)"
      @retry-attachment="store.retryFile(conversation, $event)"
      @change-model="store.changeModel(conversation, $event)"
      @change-context-limit="preferences.setContextLimit($event)"
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
