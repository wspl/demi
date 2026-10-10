<script setup lang="ts">
import type { PlaceholderText } from '@demicodes/web-ui/ui/ui-text'
import { computed, onBeforeUnmount, ref } from 'vue'
import { previewMediaType, type ContextUsage, type UserContentBlock } from '@demicodes/protocol'
import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'
import { joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import { composerCapsule } from '@demicodes/web-ui/agent/message-editor/capsules'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import type { OfflineHost } from '@demicodes/web-ui/agent/offline-host'
import type { HostChoice } from '@demicodes/web-ui/hosts/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import {
  applyAttachmentUpdate,
  arrangeCapsuleFiles,
  AttachmentUploadQueue,
  attachmentFileError,
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
  type UploadedFile,
  type UploadFile,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { applyModelChange, type ModelSettings, type ModelSettingsChange } from '@demicodes/web-ui/agent/model-selection'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'
import { demoModels, demoProviders, usageAt } from '../fixtures/catalog'
import { setGalleryContextLimit, withContextLimits } from '../fixtures/context-limits'
import { productWould } from '../product-would'
import type { SendWay, SendWhileRunning } from '@demicodes/web-ui/agent/send-way'
import { galleryMessages } from '../fixtures/send-while-running'
import { demoInstructions, demoOpenInstruction } from '../fixtures/instructions'
import type { SentMessage } from '../turn-flow'
import { createGalleryRemoteFileHosts } from '../fixtures/files'
import { galleryUploads } from '../fixtures/upload-sweep'

const props = withDefaults(
  defineProps<{
    placeholder: PlaceholderText
    running?: boolean
    compacting?: boolean
    /** What Enter does while the agent works; the gallery's preference, which Settings › General sets, without it. */
    sendWhileRunning?: SendWhileRunning
    disabled?: boolean
    /** The model catalog: `failed` tells it beside the model chip with Retry, as the product does. */
    modelLoad?: 'loading' | 'ready' | 'failed'
    /** The draft's Markdown, an attachment mark where each capsule stands; files without one follow the text. */
    draft?: string
    attachments?: ComposerAttachmentInput[]
    focused?: boolean
    attachOpen?: boolean
    dropping?: boolean
    /** The model the specimen starts with, and its Fast tier. */
    selectedProviderId?: string
    selectedModelId?: string
    serviceTierId?: string | null
    /** How full the context is; past half by default, so Compact is offered. */
    usage?: ContextUsage
    providers?: ProviderInfo[]
    models?: Record<string, ModelInfo[]>
    canConfigure?: boolean
    archived?: boolean
    hold?: string | null
    messageEdit?: MessageEditState | null
    /** The uploads this composer and its edits send files through; its own stand-in without one. */
    upload?: UploadFile
    /** A version of the draft that a later save replaced, which the composer offers to restore. */
    replaced?: string
    /** Plugins changed since the conversation opened: the composer offers a reload. */
    pluginsChanged?: boolean
    /**
     * The conversation's primary Host is an offline paired device: the
     * composer says how to start its runner, and Move to Another Host…
     * moves the specimen's conversation, whose card then goes.
     */
    offlineHost?: OfflineHost | null
    /** Compacts the specimen's conversation; without it, a toast says what the product would do. */
    onCompact?: () => void
    /** Steers the specimen's running turn; without it, a toast says what the product would do. */
    onSteer?: (content: UserContentBlock[], message: SentMessage) => void
    /** Queues a message behind the specimen's running turn; without it, a toast says what the product would do. */
    onQueue?: (content: UserContentBlock[]) => void
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
  send: [content: UserContentBlock[], message: SentMessage]
  stop: []
  configure: []
  restore: []
  retryModels: []
  'update:messageEdit': [state: MessageEditState | null]
  submitEdit: []
}>()
const draft = ref(props.draft)
const attached = ref(props.attachments.map((item) => composerAttachment(item)))
/** The files the message has, in the order of its capsules; the rest wait for an undo. */
const carried = ref<string[]>(attached.value.map((item) => item.id))
/** A draft's text and files, as the replaced version keeps them. */
interface DraftVersion {
  text: string
  files: ComposerAttachment[]
}
/** The version a later save replaced, as the product's backend keeps it. */
const replacedVersion = ref<DraftVersion | null>(
  props.replaced ? { text: props.replaced, files: [] } : null,
)
/** Whether the reload is offered, and whether one is under way, as the product's summary and store hold them. */
const pluginsChanged = ref(props.pluginsChanged === true)
const reloading = ref(false)
let reloadTimer = 0
/** Reloads after a beat, as the product's tree closes and reopens; the offer goes. */
function reloadPlugins() {
  reloading.value = true
  reloadTimer = window.setTimeout(() => {
    reloading.value = false
    pluginsChanged.value = false
  }, 600)
}
/** The offline primary Host's card, until a move takes the conversation to another Host. */
const offlineHost = ref(props.offlineHost ?? null)
const moving = ref(false)
let moveTimer = 0
/** Moves after a beat, as the product's switch commits, and says where the product would run it. */
function moveHost(host: HostChoice) {
  moving.value = true
  moveTimer = window.setTimeout(() => {
    moving.value = false
    const to = host.kind === 'cloud'
      ? 'Cloud'
      : offlineHost.value?.devices.find((device) => device.id === host.id)?.name
    offlineHost.value = null
    productWould(`Move the Conversation to ${to}`)
  }, 600)
}
/** Counts the drafts shown from outside, here the restored ones. */
const shown = ref(0)
/** Compact from the meter's card: the specimen's own conversation, or a toast when it has none. */
function compact() {
  if (props.onCompact) {
    props.onCompact()
    return
  }
  productWould('Compact the Conversation')
}
const composer = ref<InstanceType<typeof SessionComposer>>()
const uploads = new AttachmentUploadQueue()
const host = galleryUploads()
/** The gallery's stand-in for the backend, unless the page that holds this composer brings its own. */
const upload: UploadFile = (file, options) => (props.upload ?? host.upload)(file, options)
/**
 * The bytes of each file the composer carries, and what its upload answered
 * once it has. A file the specimen starts with stands in without bytes, under
 * its name, so that its upload can go again as a picked one's does.
 */
const picked = new Map<string, File>(
  attached.value.flatMap((item) =>
    isComposerFile(item) ? [[item.id, new File([], item.name, { type: previewMediaType(item.name) ?? '' })]] : [],
  ),
)
const answers = new Map<string, UploadedFile>()
/** The specimen's model settings, which its model menu changes as the product's draft does. */
const settings = ref<ModelSettings>({
  providerId: props.selectedProviderId,
  modelId: props.selectedModelId,
  thinkingEffort: 'medium',
  serviceTierId: props.serviceTierId ?? null,
})
/** What the composer takes of the specimen's props: the model it starts with is the settings'. */
const composerProps = computed(() => {
  const {
    selectedProviderId: _provider,
    selectedModelId: _model,
    serviceTierId: _tier,
    onCompact: _compact,
    onSteer: _steer,
    onQueue: _queue,
    sendWhileRunning: _sendWhileRunning,
    offlineHost: _offlineHost,
    ...rest
  } = props
  return rest
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
  picked.delete(id)
  answers.delete(id)
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
 * The files the message now has, in the order of its capsules; the rest wait
 * for an undo, as the product's do.
 */
function arrange(ids: string[]) {
  const next = arrangeCapsuleFiles(attached.value, carried.value, ids)
  carried.value = [...ids]
  attached.value = next.carried
  for (const item of next.stopped) {
    uploads.cancel(item.id)
  }
  // A file whose upload a deleted capsule stopped starts over; one still uploading goes on.
  for (const item of next.resumed) {
    if (isComposerFile(item) && item.phase !== 'ready' && !uploads.has(item.id)) {
      startUpload(item.id)
    }
  }
}

/** A sent file as the transcript records it: a record at a made-up Host path, a picture before it, or a device's file. */
function sentBlocks(item: ComposerAttachment): UserContentBlock[] {
  if (item.kind === 'reference') {
    return [{ type: 'reference', reference: encodeRemoteReference(item.host, item.path) }]
  }
  const answer = answers.get(item.id)
  const record: UserContentBlock = {
    type: 'attachment',
    name: item.name,
    path: `/home/demi/.demi/attachments/gallery/${item.name}`,
    mediaType: answer?.mediaType ?? 'application/octet-stream',
    sizeBytes: picked.get(item.id)?.size ?? 0,
    sha256: answer?.sha256 ?? '0'.repeat(64),
    ...(item.snippet ? { snippet: item.snippet } : {}),
  }
  return item.src ? [{ type: 'image', source: { type: 'url', url: item.src } }, record] : [record]
}

/** Sends the draft the way the composer chose: at once, as a steer, or to the queue. */
function submit(way: SendWay) {
  const sent = carried.value.flatMap((id) => attached.value.filter((item) => item.id === id))
  const content = joinMessageContent(draft.value, sent.map(sentBlocks))
  if (!content.length) {
    return
  }
  // The message as the composer shows it, which the page holds until the server confirms it.
  const message: SentMessage = { text: draft.value, attachments: sent }
  draft.value = ''
  carried.value = []
  while (attached.value.length) {
    forget(attached.value[0]!.id, false)
  }
  if (way === 'steer') {
    if (props.onSteer) {
      props.onSteer(content, message)
    } else {
      productWould('Steer the Running Turn')
    }
  } else if (way === 'queue') {
    if (props.onQueue) {
      props.onQueue(content)
    } else {
      productWould('Queue the Message')
    }
  } else {
    emit('send', content, message)
  }
}

function startUpload(id: string) {
  const file = picked.get(id)
  if (!file) {
    return
  }
  // The stand-in never fails; were it to, the file would keep its capsule with Retry, as the product's does.
  uploads.start(id, async (signal, report) => {
    const answer = await upload(file, { signal, progress: report })
    answers.set(id, answer)
    const item = attached.value.find((each) => each.id === id)
    if (item && isComposerFile(item)) {
      item.snippet = answer.snippet
    }
  }, (update) => applyUpdate(id, update)).catch(() => applyUpdate(id, { phase: 'failed' }))
}

/** Takes files and puts their capsules in the message, where the composer said they would land. */
function addFiles(files: File[]) {
  const taken: ComposerAttachment[] = []
  for (const file of files) {
    if (attachmentFileError(file, composerFileNames(attached.value))) {
      continue
    }
    const item = composerAttachmentFromFile(file)
    picked.set(item.id, file)
    attached.value.push(item)
    const held = attached.value.find((each) => each.id === item.id)!
    taken.push(held)
    startUpload(item.id)
  }
  composer.value?.insertCapsules(taken.map(composerCapsule))
}

function retry(id: string) {
  const item = attached.value.find((file) => file.id === id)
  if (item && isComposerFile(item) && item.phase === 'failed') {
    startUpload(id)
  }
}

const remoteHosts = createGalleryRemoteFileHosts()
const remotePicker = ref<InstanceType<typeof RemoteFilePicker>>()
function attachRemote(file: { host: string; path: string }) {
  const error = remoteAttachmentError(file.path, file.host, attached.value)
  if (!error) {
    const item = composerRemoteAttachment(file)
    attached.value.push(item)
    composer.value?.insertCapsules([composerCapsule(item)])
  }
}
/**
 * Exchanges the replaced version with the draft, as the product's restore
 * does: the draft it displaces is offered in its place, unless it is empty,
 * and what the composer held for an undo goes.
 */
function restoreReplaced() {
  const restored = replacedVersion.value
  if (!restored) {
    return
  }
  const displaced: DraftVersion = {
    text: draft.value,
    files: carried.value.flatMap((id) => attached.value.filter((item) => item.id === id)),
  }
  for (const item of attached.value.filter((file) => !carried.value.includes(file.id))) {
    forget(item.id, true)
  }
  replacedVersion.value = displaced.text.trim() || displaced.files.length ? displaced : null
  attached.value = [...restored.files]
  carried.value = restored.files.map((file) => file.id)
  draft.value = restored.text
  shown.value += 1
}

function changeModel(change: ModelSettingsChange) {
  settings.value = applyModelChange(settings.value, change)
}
onBeforeUnmount(() => {
  window.clearTimeout(reloadTimer)
  window.clearTimeout(moveTimer)
  uploads.cancelAll()
  host.release()
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
    ref="composer"
    v-bind="composerProps"
    v-model:draft="draft"
    :send-while-running="props.sendWhileRunning ?? galleryMessages.sendWhileRunning"
    @update:message-edit="emit('update:messageEdit', $event)"
    @submit-edit="emit('submitEdit')"
    :attachments="attached"
    :upload="upload"
    :providers="props.providers ?? demoProviders"
    :models="withContextLimits(props.models ?? demoModels)"
    :can-configure="canConfigure !== false"
    :archived="archived"
    :hold="hold"
    :model-settings="settings"
    :usage="props.usage ?? usageAt(0.62)"
    :instructions="demoInstructions"
    :replaced="replacedVersion && { markdown: replacedVersion.text, fileNames: replacedVersion.files.map((file) => file.name) }"
    :draft-shown="shown"
    :plugins-changed="pluginsChanged"
    :reloading="reloading"
    :offline-host="offlineHost && { ...offlineHost, moving }"
    @move-host="moveHost"
    remote-files
    @reload-plugins="reloadPlugins"
    @restore-replaced="restoreReplaced"
    @dismiss-replaced="replacedVersion = null"
    @submit="submit"
    @configure="emit('configure')"
    @restore="emit('restore')"
    @retry-models="emit('retryModels')"
    @add-files="addFiles"
    @attach-remote="remotePicker?.open()"
    @arrange-attachments="arrange"
    @retry-attachment="retry"
    @change-model="changeModel"
    @change-context-limit="setGalleryContextLimit"
    @stop="emit('stop')"
    @compact="compact"
    @open-instruction="demoOpenInstruction($event, productWould)"
  />
  <RemoteFilePicker
    ref="remotePicker"
    :overlay-store="appOverlayStore"
    :hosts="remoteHosts"
    @select="attachRemote"
  />
</template>
