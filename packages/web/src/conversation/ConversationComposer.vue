<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { baseName } from '@demicodes/utils'

import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'
import { loadedInstructions } from '@demicodes/web-ui/agent/instructions'
import { latestBlocks } from '@demicodes/web-ui/agent/history'

import { composerRemoteAttachment } from '@demicodes/web-ui/agent/message-input/attachments'
import { composerCapsule } from '@demicodes/web-ui/agent/message-editor/capsules'
import { savedPreview, type SavedRow } from '@demicodes/web-ui/agent/saved'
import { turnStartingMessage, type SendWay } from '@demicodes/web-ui/agent/send-way'
import { executionFor, hostDeviceOptions, moveLock, primaryHostOf } from '../targets/execution'
import type { OfflineHost } from '@demicodes/web-ui/agent/offline-host'
import type { HostChoice } from '@demicodes/web-ui/hosts/types'
import type { MessageFileTarget } from '@demicodes/web-ui/markdown/types'
import { useConversations } from './store'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { hostKindOf } from '../state/hosts'
import { usePreferences } from '../state/preferences'
import { placesFor } from '../devices/files'
import { conversationHostSource } from './files'
import { claimStoredFile, uploadAttachment } from '../api/uploads'
import type { Conversation } from '../state/types'
import { cliPackageOf } from '../state/catalog'
import { useSettingsAddress } from '../settings/address'
import { useContextCard } from './context-card'

const props = defineProps<{
  conversation: Conversation
  /** Shows a Host file in the work panel; absent while no plugin shows files. */
  openFile?: (target: MessageFileTarget) => void
}>()
const emit = defineEmits<{
  /** A Host the offline primary Host's card chose to move the conversation to. */
  moveHost: [host: HostChoice]
}>()
const store = useConversations()
const resources = useResources()
const preferences = usePreferences()
const product = useProduct()
const settings = useSettingsAddress()

const composer = ref<InstanceType<typeof SessionComposer>>()
// New pressed on this very draft: the message takes the focus again.
watch(() => store.composerFocusRequests, () => composer.value?.focusDraft())
/**
 * The message that starts a turn is on its way: the turn is as good as
 * running, so Stop shows from the send on, never a disabled Send first, and
 * a message sent meanwhile steers it or waits in the queue.
 */
const startingTurn = computed(() =>
  turnStartingMessage(props.conversation.phase, props.conversation.pendingSends) !== undefined,
)

/** The store takes the files; their capsules go into the message where the composer said they would land. */
function addFiles(files: File[]) {
  composer.value?.insertCapsules(store.addFiles(props.conversation, files).map(composerCapsule))
}

/** How a message pasted into the draft brings its files back, as the store takes them. */
const paste = computed(() => store.pasteHost(props.conversation))

/** The version of the draft a later save replaced, which the composer offers to restore. */
const replaced = computed(() => {
  const version = props.conversation.savedDraft?.replaced
  return version
    ? {
        markdown: version.text,
        fileNames: version.files.map((file) =>
          file.type === 'upload' ? file.fileName : baseName(file.path),
        ),
      }
    : null
})

/** The messages saved for later as the list under the composer shows them, newest first. */
const savedRows = computed<SavedRow[]>(() =>
  (props.conversation.saved?.messages ?? []).map((message) => (
    { id: message.id, preview: savedPreview(message.text), attachmentCount: message.files.length }
  )),
)
/** The list stands open while this composer shows; another conversation's starts closed. */
const savedOpen = ref(false)
watch(() => props.conversation.id, () => {
  savedOpen.value = false
})

function openProviders() {
  void settings.open('models')
}
/** The composer sends only what can go: a message that waits for the models goes once they load. */
function send(way: SendWay) {
  void store.send(props.conversation, way)
}

/** What the model holds of the user's and the project's instructions, which the context card lists. */
const instructions = computed(() => loadedInstructions(latestBlocks(props.conversation.history), props.conversation.instructions))

const card = useContextCard(() => props.openFile)
/** What the context card shows of the usage limits of the account the conversation's model runs on. */
const limits = computed(() => card.limitsOf(props.conversation.model.providerId))

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
/**
 * The primary Host when it is a paired device whose runner is not
 * connected, with what its card's Move to Another Host… lists once the
 * conversation's work is idle; a Host chosen there is the header's to move
 * the conversation to.
 */
const offlineHost = computed<OfflineHost | null>(() => {
  const primary = execution.value
  if (primary.kind !== 'device' || primary.state !== 'offline') {
    return null
  }
  const start = resources.deviceById(primary.deviceId)?.start
  return start
    ? {
        name: primary.name,
        start,
        primaryHost: primaryHostOf(primary),
        devices: hostDeviceOptions(resources.devices),
        chooseDirectory: props.conversation.target.kind === 'workspace',
        locked: moveLock(props.conversation, store.pendingChanges),
      }
    : null
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
      canWake: hostKindOf(device) === 'cloud',
      kind: hostKindOf(device),
      source: conversationHostSource(props.conversation.id, host.id, host.id === primary.deviceId),
      places: placesFor(device, resources.projects),
      cwd: cwd ?? undefined,
    }
  })
})
const remotePicker = ref<InstanceType<typeof RemoteFilePicker>>()
function attachRemote(file: { deviceId: string; host: string; path: string }) {
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
      :claim="claimStoredFile"
      :paste="paste"
      @submit-edit="store.submitEdit(conversation)"
      placeholder="Ask Demi…"
      focus-on-show
      :running="conversation.phase === 'running' || startingTurn"
      :compacting="conversation.phase === 'compacting'"
      :send-while-running="preferences.sendWhileRunning"
      :can-configure="resources.canConfigure"
      :attachments="
        conversation.files.filter(
          (file) => !conversation.pendingSends.some((pending) => pending.fileIds.includes(file.id)),
        )
      "
      :providers="resources.providerInfos"
      :models="resources.models"
      :model-settings="conversation.model"
      :usage="conversation.contextUsage"
      :instructions="instructions"
      :limits="limits"
      @refresh-limits="card.refreshLimits(conversation.model.providerId, $event)"
      :remote-files="remoteHosts.length > 0"
      :archived="conversation.archived"
      :hold="hold"
      :offline-host="offlineHost"
      @move-host="emit('moveHost', $event)"
      :replaced="replaced"
      :draft-shown="conversation.draftShown"
      :plugins-changed="conversation.pluginsChanged"
      :reloading="store.pendingChanges.includes(conversation.id)"
      :saved="savedRows"
      v-model:saved-open="savedOpen"
      @save-for-later="store.saveForLater(conversation)"
      @edit-saved="store.editSaved(conversation, $event)"
      @send-saved="(id, way) => store.sendSaved(conversation, id, way)"
      @delete-saved="store.deleteSaved(conversation, $event)"
      @restore-replaced="store.restoreReplaced(conversation)"
      @dismiss-replaced="store.dismissReplaced(conversation)"
      @reload-plugins="store.reloadPlugins(conversation)"
      @submit="send"
      @configure="openProviders"
      @restore="store.restore([conversation.id])"
      @add-files="addFiles"
      @attach-remote="remotePicker?.open()"
      @arrange-attachments="store.arrangeFiles(conversation, $event)"
      @retry-attachment="store.retryFile(conversation, $event)"
      @change-model="store.changeModel(conversation, $event)"
      @change-context-limit="preferences.setContextLimit($event)"
      @stop="store.stop(conversation)"
      @compact="store.compact(conversation)"
      @open-instruction="card.openInstruction"
    />
    <RemoteFilePicker
      ref="remotePicker"
      :overlay-store="appOverlayStore"
      :hosts="remoteHosts"
      @select="attachRemote"
    />
  </div>
</template>
