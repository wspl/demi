<script setup lang="ts">
import { keptComputed } from '@demicodes/web-ui/infra/keep-same'
import { computed, ref, watch } from 'vue'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'

import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'
import { loadedInstructions } from '@demicodes/web-ui/agent/instructions'
import { latestBlocks } from '@demicodes/web-ui/agent/history'
import WaitingComments from '@demicodes/web-ui/agent/comments/WaitingComments.vue'
import { inReadingOrder } from '@demicodes/web-ui/agent/comments/comments'
import { changeEditComments, editComments } from '@demicodes/web-ui/agent/message-editing'
import { refusedForSize } from '@demicodes/web-ui/agent/session-status'
import { isCompacting } from '@demicodes/web-ui/agent/conversation-status'

import { composerRemoteAttachment } from '@demicodes/web-ui/agent/message-input/attachments'
import { composerCapsule } from '@demicodes/web-ui/agent/message-editor/capsules'
import { savedParts, type SavedRow } from '@demicodes/web-ui/agent/saved'
import type { DraftFile } from '../api/generated/web-api'
import { turnStartingMessage, type SendWay } from '@demicodes/web-ui/agent/send-way'
import { executionFor, hostDeviceOptions, moveLock, primaryHostOf } from '../targets/execution'
import type { OfflineHost } from '@demicodes/web-ui/agent/offline-host'
import type { HostChoice } from '@demicodes/web-ui/hosts/types'
import type { MessageFileTarget } from '@demicodes/web-ui/markdown/types'
import { useConversations } from './store'
import { useHeldList, useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { hostKindOf } from '../state/hosts'
import { usePreferences } from '../state/preferences'
import { placesFor } from '../devices/files'
import { conversationHostSource } from './files'
import { claimStoredFile, uploadAttachment } from '../api/uploads'
import type { Conversation, ProductAttachment } from '../state/types'
import { cliPackageOf } from '../state/catalog'
import { useSettingsAddress } from '../settings/address'
import { useContextCard } from './context-card'
import { useConversationWorkflows } from './workflows'
import { workflowProfiles } from '@demicodes/web-ui/agent/workflow-menu'
import { t } from './locales/text'

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
// The Workflow menu's profiles come with the subagent settings, which the composer holds while it shows.
useHeldList('subagents')
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

/**
 * A file of the composer with where it is once a message can name it without
 * its bytes, which a copy of the message carries: its blob once uploaded, or
 * its device.
 */
function referenced(file: ProductAttachment): ProductAttachment {
  if (file.kind === 'reference') {
    return { ...file, reference: { type: 'device', deviceId: file.deviceId } }
  }
  return file.upload ? { ...file, reference: { type: 'stored', sha256: file.upload.sha256, mediaType: file.upload.mediaType } } : file
}

/** The composer's files, those of a message on its way left out. */
const attachments = keptComputed(() => props.conversation.files.filter(
  (file) => !props.conversation.pendingSends.some((pending) => pending.fileIds.includes(file.id)),
).map(referenced))

/** The store takes the files; their capsules go into the message where the composer said they would land. */
function addFiles(files: File[]) {
  composer.value?.insertCapsules(store.addFiles(props.conversation, files).map(composerCapsule))
}

/** How a message pasted into the draft brings its files back, as the store takes them. */
const paste = computed(() => store.pasteHost(props.conversation))

/**
 * The messages saved for later as the list under the composer shows them,
 * newest first, each saved, or saving while the backend has not taken it or
 * its edit: the page's write of it is on its way, or waits in the queue the
 * web browser keeps, whose text the row shows.
 */
const savedRows = keptComputed(savedRowsNow)
function savedRowsNow(): SavedRow[] {
  const record = props.conversation.record
  const shown = record.shown.value?.saved ?? []
  const kept = new Set(record.read.value?.saved.map((message) => message.id))
  const queued = (id: string) => props.conversation.savedWrites.findLast((write) => write.id === id)
  const row = (id: string, text: string, files: DraftFile[], state: SavedRow['state'], cut = false): SavedRow => {
    const { capsules } = store.savedMessage(props.conversation, { text, files })
    return { id, parts: savedParts(text, capsules, cut), state }
  }
  // A saved message's row shows its opening, the start of its text, with its
  // files where their marks stand; an opening shorter than the message ends
  // in an ellipsis, the files past it after.
  return [
    // A save the page lists no more, as one whose write failed, still stands.
    ...props.conversation.savedWrites
      .filter((write) => write.kind === 'save' && !shown.some((message) => message.id === write.id))
      .reverse()
      .map((write) => row(write.id, write.text, write.files, 'saving')),
    ...shown.map((message) => {
      const write = queued(message.id)
      return write
        ? row(message.id, write.text, write.files, 'saving')
        : row(message.id, message.opening, message.files, kept.has(message.id) ? 'saved' : 'saving', message.length > [...message.opening].length)
    }),
  ]
}
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

/** The comments waiting in the draft, in the order the conversation reads them, as the pill lists them. */
const waitingComments = computed(() =>
  inReadingOrder(props.conversation.comments, latestBlocks(props.conversation.history).map((block) => block.id)),
)
const commenting = computed(() => store.commenting(props.conversation))

/**
 * The edit of a sent message the composer shows, whose own comments the pill
 * lists in place of the draft's; none while no such edit is shown.
 */
const sentEdit = computed(() => {
  const edit = props.conversation.messageEdit
  return edit?.kind === 'message' && edit.phase !== 'regenerating' ? edit : null
})
/** The comments the pill lists: the shown edit's, else the draft's. */
const carriedComments = keptComputed(() => {
  if (sentEdit.value) {
    return editComments(sentEdit.value)
  }
  return props.conversation.messageEdit?.kind === 'saved' ? [] : waitingComments.value
})
/** Changes the shown edit's comments by `change`. */
function changeEdit(change: Parameters<typeof changeEditComments>[1]): void {
  if (sentEdit.value) {
    props.conversation.messageEdit = changeEditComments(sentEdit.value, change)
  }
}

/** What the model holds of the user's and the project's instructions, which the context card lists. */
const instructions = computed(() => loadedInstructions(latestBlocks(props.conversation.history), props.conversation.instructions))
/** The unfinished turn failed for its size: Compact is offered at any share. */
const sizeRefused = computed(() => refusedForSize(latestBlocks(props.conversation.history)))

const card = useContextCard(() => props.openFile)
/** What the context card shows of the usage limits of the account the conversation's model runs on. */
const limits = keptComputed(() =>
  card.limitsOf(props.conversation.model.providerId, props.conversation.contextUsage?.account ?? null),
)
/** What the conversation cost, which the context card shows. */
const cost = keptComputed(() => store.costShown(props.conversation))

const execution = keptComputed(() => executionFor(props.conversation))

/**
 * The Workflow menu's choice, as the metadata shows it with the changes on
 * their way, and the project's workflows; none before the first send makes
 * the record.
 */
const record = computed(() => props.conversation.metadata.shown.value)
const workflowList = useConversationWorkflows(() => props.conversation)
/**
 * The record's choice, with the changes on their way; a new conversation's
 * own until its record is made, whatever the channel already brought of it.
 */
const workflow = keptComputed(() => ({
  choice: props.conversation.persistence === 'synced' && record.value ? record.value.workflow : props.conversation.workflow,
  list: workflowList.value,
}))
/** The profiles the Workflow menu's choice offers now, whose subagents the cost's Profiles part shows. */
const costProfiles = keptComputed(() => workflowProfiles(
  workflow.value.choice,
  workflow.value.list,
  product.snapshot?.subagents ?? { enabled: false, profiles: [] },
))
// A reset holds the conversations that cannot work without the Cloud: it is
// their target, or where their provider's process runs. The account snapshot
// the page polls says when it ends, and the input returns by itself.
const usesCloud = computed(() =>
  execution.value.kind === 'cloud' ||
  cliPackageOf(product.catalog, props.conversation.model.providerId) !== null,
)
const hold = computed(() =>
  usesCloud.value && product.snapshot?.cloud?.state === 'resetting'
    ? t('composer.cloudResetting')
    : null,
)
/**
 * The primary Host when it is a paired device whose runner is not
 * connected, with what its card's Move to Another Host… lists once the
 * conversation's work is idle; a Host chosen there is the header's to move
 * the conversation to.
 */
const offlineHost = keptComputed<OfflineHost | null>(() => {
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
const remoteHosts = keptComputed(() => {
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
    ...(props.conversation.attachedHosts.shown.value ?? [])
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
        : props.conversation.attachedHosts.shown.value?.find(
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
      v-model:draft="conversation.draft"
      v-model:message-edit="conversation.messageEdit"
      :upload="uploadAttachment"
      :claim="claimStoredFile"
      :paste="paste"
      @submit-edit="store.submitEdit(conversation, $event)"
      @save-edit="store.saveSavedEdit(conversation)"
      focus-on-show
      :running="conversation.phase === 'running' || startingTurn"
      :compacting="isCompacting(conversation.phase)"
      :send-while-running="preferences.sendWhileRunning"
      :can-configure="resources.canConfigure"
      :attachments="attachments"
      :providers="resources.providerInfos"
      :models="resources.models"
      :model-settings="conversation.model"
      :usage="conversation.contextUsage"
      :size-refused="sizeRefused"
      :instructions="instructions"
      :limits="limits"
      :cost="cost"
      :cost-profiles="costProfiles"
      @refresh-limits="card.refreshLimits(conversation.model.providerId, conversation.contextUsage?.account ?? null)"
      :remote-files="remoteHosts.length > 0"
      :archived="conversation.archived"
      :hold="hold"
      :offline-host="offlineHost"
      @move-host="emit('moveHost', $event)"
      :draft-shown="conversation.draftShown"
      :plugins-changed="conversation.pluginsChanged"
      :reloading="store.pendingChanges.includes(conversation.id)"
      :saved="savedRows"
      :workflow="workflow"
      v-model:saved-open="savedOpen"
      @save-for-later="store.saveForLater(conversation)"
      @edit-saved="store.editSaved(conversation, $event)"
      @send-saved="(id, way) => store.sendSaved(conversation, id, way)"
      @delete-saved="store.deleteSaved(conversation, $event)"
      @reload-plugins="store.reloadPlugins(conversation)"
      @submit="send"
      @configure="openProviders"
      @restore="store.restore([conversation.id])"
      @add-files="addFiles"
      @attach-remote="remotePicker?.open()"
      @arrange-attachments="store.arrangeFiles(conversation, $event)"
      @retry-attachment="store.retryFile(conversation, $event)"
      @change-model="store.changeModel(conversation, $event)"
      @change-workflow="void store.changeWorkflow(conversation.id, $event)"
      @change-context-limit="preferences.setContextLimit($event)"
      :conversation-id="conversation.id"
      @stop="store.stop(conversation)"
      @compact="store.compact(conversation)"
      @open-instruction="card.openInstruction"
    >
      <template v-if="carriedComments.length > 0" #carried>
        <WaitingComments
          v-if="sentEdit"
          :comments="carriedComments"
          :readonly="sentEdit.phase !== 'editing'"
          @edit="(id, text) => changeEdit((comments) => comments.map((each) => (each.id === id ? { ...each, text } : each)))"
          @remove="(id) => changeEdit((comments) => comments.filter((each) => each.id !== id))"
          @clear="changeEdit(() => [])"
        />
        <WaitingComments
          v-else
          :comments="carriedComments"
          @edit="(id, text) => commenting.edit(id, text)"
          @remove="commenting.remove($event)"
          @clear="store.clearComments(conversation)"
        />
      </template>
    </SessionComposer>
    <RemoteFilePicker
      ref="remotePicker"
      :overlay-store="appOverlayStore"
      :hosts="remoteHosts"
      @select="attachRemote"
    />
  </div>
</template>
