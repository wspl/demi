<script setup lang="ts">
import type { PlaceholderText } from '@demicodes/web-ui/ui/ui-text'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { previewMediaType, type ContextUsage, type UserContentBlock } from '@demicodes/protocol'
import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'
import { isEmptyMessage, joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import { composerCapsule } from '@demicodes/web-ui/agent/message-editor/capsules'
import {
  missingFilesToast,
  pasteMessage,
  type CopiedStoredFile,
  type PasteHost,
} from '@demicodes/web-ui/agent/message-input/copied-message'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { createId } from '@demicodes/utils'
import type { DraftRow } from '@demicodes/web-ui/agent/drafts'
import { draftPreview } from '@demicodes/web-ui/agent/message-input/draft-preview'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import RemoteFilePicker from '@demicodes/web-ui/files/RemoteFilePicker.vue'
import type { OfflineHost } from '@demicodes/web-ui/agent/offline-host'
import type { HostChoice } from '@demicodes/web-ui/hosts/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import {
  applyAttachmentUpdate,
  arrangeCapsuleFiles,
  attachingUpdate,
  AttachmentUploadQueue,
  attachmentFileError,
  composerAttachment,
  composerAttachmentFromFile,
  composerRemoteAttachment,
  encodeRemoteReference,
  filePreviewUrl,
  isComposerFile,
  type AttachmentUploadUpdate,
  type ComposerAttachment,
  type ComposerAttachmentInput,
  type ComposerFileAttachment,
  type UploadedFile,
  type UploadFile,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { applyModelChange, type ModelSettings, type ModelSettingsChange } from '@demicodes/web-ui/agent/model-selection'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'
import { demoProviders, usageAt } from '../fixtures/catalog'
import { composerModels, setGalleryContextLimit, withContextLimits } from '../fixtures/context-limits'
import { productWould } from '../product-would'
import { chooseSendWay, type SendWay, type SendWhileRunning } from '@demicodes/web-ui/agent/send-way'
import { galleryMessages } from '../fixtures/send-while-running'
import { demoInstructions, demoOpenInstruction } from '../fixtures/instructions'
import type { SentMessage, RecallComposer } from '../turn-flow'
import { createGalleryRemoteFileHosts } from '../fixtures/files'
import { galleryUploads } from '../fixtures/upload-sweep'
import type { SeedDraft } from '../fixtures/drafts'

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
    onQueue?: (content: UserContentBlock[], message: SentMessage) => void
    /** The specimen's flow, whose Recall gives a message back into this composer, as the product's does. */
    flow?: { attachComposer(host: RecallComposer): () => void }
    /** The drafts the specimen starts with, newest first; Save as Draft adds to them. */
    drafts?: SeedDraft[]
    /** The drafts' list starts open. */
    draftsOpen?: boolean
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
  /** The offline primary Host's card moved the conversation to `host`, named `name`. */
  moved: [host: HostChoice, name: string]
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
/** A saved draft as the product's server keeps it: the message and its files. */
interface SavedDraft {
  id: string
  markdown: string
  files: ComposerAttachment[]
}
const savedDrafts = ref<SavedDraft[]>(
  (props.drafts ?? []).map((seed) => ({
    id: createId(),
    markdown: seed.text,
    files: (seed.files ?? []).map((item) => composerAttachment(item)),
  })),
)
const draftsOpen = ref(props.draftsOpen === true)
const draftRows = computed<DraftRow[]>(() => savedDrafts.value.map((saved) => ({
  id: saved.id,
  preview: draftPreview(saved.markdown, saved.files.map((file) => file.name)),
  attachmentCount: saved.files.length,
})))
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
/**
 * The offline primary Host's card, until a move takes the conversation to
 * another Host or the specimen brings the device back.
 */
const offlineHost = ref(props.offlineHost ?? null)
// Only a device that comes or goes resets the card: a specimen's inline value is new on every render.
watch(() => !!props.offlineHost, () => {
  offlineHost.value = props.offlineHost ?? null
})
/** The card goes with the move, as the product's does, and a toast says where the product would run it. */
function moveHost(host: HostChoice) {
  const to = host.kind === 'cloud'
    ? 'Cloud'
    : offlineHost.value?.devices.find((device) => device.id === host.id)?.name
  offlineHost.value = null
  emit('moved', host, to ?? host.kind)
  productWould(`Move the Conversation to ${to}`)
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
/** Each file's upload under way, which a message sent meanwhile waits for. */
const transfers = new Map<string, Promise<void>>()
/** The stored files pasted messages name, by their files' ids, with the picture each copy carried. */
const pastedClaims = new Map<string, { file: CopiedStoredFile; picture: File | null }>()
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
    flow: _flow,
    sendWhileRunning: _sendWhileRunning,
    offlineHost: _offlineHost,
    drafts: _drafts,
    draftsOpen: _draftsOpen,
    ...rest
  } = props
  return rest
})

/** Pictures of sent files stay loadable while the transcript shows them; they go with the page. */
const sentPictures: string[] = []

/** Lets a file go: its upload stops, unless a sent message `keepsUpload` waits for it. */
function forget(id: string, release: boolean, keepsUpload = false) {
  if (!keepsUpload) {
    uploads.cancel(id)
    picked.delete(id)
    answers.delete(id)
    pastedClaims.delete(id)
  }
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
  // The gallery's devices go by their names.
  if (item.kind === 'reference') {
    return [{ type: 'reference', reference: encodeRemoteReference(item.host, item.path, item.host) }]
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

/**
 * Sends the draft the way the composer chose: at once, as a steer, or to the
 * queue. A message whose files still upload shows at once and waits for
 * them, as the product's does (`product.md` § Writing a message); a steer or
 * a queued message is handed on once they are in.
 */
function submit(way: SendWay) {
  const sent = carried.value.flatMap((id) => attached.value.filter((item) => item.id === id))
  if (!handOn(draft.value, sent, way)) {
    return
  }
  draft.value = ''
  carried.value = []
  // What the composer carried for an undo goes with the message.
  while (attached.value.length) {
    forget(attached.value[0]!.id, false)
  }
}

/**
 * Keeps the message as a draft at the top of the list and empties the
 * composer, as the product's Save as Draft does: its files go with it, their
 * uploads going on. A composer holding nothing saves nothing.
 */
function saveDraft() {
  if (isEmptyMessage(draft.value, carried.value.length)) {
    return
  }
  const files = carried.value.flatMap((id) => attached.value.filter((item) => item.id === id))
  // What the composer held only for an undo goes.
  for (const item of attached.value.filter((file) => !carried.value.includes(file.id))) {
    forget(item.id, true)
  }
  savedDrafts.value = [
    { id: createId(), markdown: draft.value, files },
    ...savedDrafts.value,
  ]
  draft.value = ''
  carried.value = []
  attached.value = []
  composer.value?.focusDraft()
}

/** Takes a draft off the list, and gives it back at its place. */
function takeDraft(id: string): { saved: SavedDraft; index: number } | undefined {
  const index = savedDrafts.value.findIndex((saved) => saved.id === id)
  const saved = savedDrafts.value[index]
  if (!saved) {
    return undefined
  }
  savedDrafts.value = savedDrafts.value.filter((each) => each.id !== id)
  return { saved, index }
}

/**
 * Puts a draft in the composer and takes it off the list; a composer holding
 * a message saves that one as a draft first, so nothing is lost.
 */
function editDraft(id: string) {
  const taken = takeDraft(id)
  if (!taken) {
    return
  }
  saveDraft()
  attached.value = [...taken.saved.files]
  carried.value = taken.saved.files.map((file) => file.id)
  draft.value = taken.saved.markdown
  shown.value += 1
  composer.value?.focusDraft()
}

/** Sends a draft as the composer would, steering or queueing while the agent works, and takes it off the list. */
function sendDraft(id: string) {
  const taken = takeDraft(id)
  if (!taken) {
    return
  }
  const way = chooseSendWay(
    props.running === true || props.compacting === true,
    props.sendWhileRunning ?? galleryMessages.sendWhileRunning,
    false,
  )
  handOn(taken.saved.markdown, taken.saved.files, way)
  if (way === 'send') {
    productWould('Send the Draft')
  }
}

/** Removes a draft at once, with Undo in a toast that puts it back at its place. */
function deleteDraft(id: string) {
  const taken = takeDraft(id)
  if (!taken) {
    return
  }
  showToast({
    title: 'Draft Deleted',
    tone: 'neutral',
    action: {
      label: 'Undo',
      run: () => {
        const next = [...savedDrafts.value]
        next.splice(Math.min(taken.index, next.length), 0, taken.saved)
        savedDrafts.value = next
      },
    },
  })
}

/**
 * Hands the message `text` with the files `sent` on the way the composer
 * chose, and lets its files go from the composer, those still uploading
 * going on for it; false when it has nothing to send.
 */
function handOn(text: string, sent: ComposerAttachment[], way: SendWay): boolean {
  const content = joinMessageContent(text, sent.map(sentBlocks))
  if (!content.length) {
    return false
  }
  const waiting = sent.filter((item) => isComposerFile(item) && item.phase !== 'ready')
  const ready = waiting.length
    ? Promise.all(waiting.map((item) => transfers.get(item.id))).then(() => {
        if (waiting.some((item) => !answers.has(item.id))) {
          throw new Error('A file of this message did not upload.')
        }
        const blocks = joinMessageContent(text, sent.map(sentBlocks))
        for (const item of waiting) {
          picked.delete(item.id)
          answers.delete(item.id)
          pastedClaims.delete(item.id)
        }
        return blocks
      })
    : undefined
  // The message as the composer shows it, which the page holds until the server confirms it.
  const message: SentMessage = { text, attachments: sent, ready }
  for (const item of sent) {
    forget(item.id, false, waiting.includes(item))
  }
  const hand = (to: (content: UserContentBlock[], message: SentMessage) => void) =>
    ready ? void ready.then((blocks) => to(blocks, message), () => productWould('Retry the Upload')) : to(content, message)
  if (way === 'steer') {
    if (props.onSteer) {
      hand(props.onSteer)
    } else {
      productWould('Steer the Running Turn')
    }
  } else if (way === 'queue') {
    if (props.onQueue) {
      hand(props.onQueue)
    } else {
      productWould('Queue the Message')
    }
  } else {
    emit('send', content, message)
  }
  return true
}

/**
 * Recall gives a message back into this composer when it is empty, as
 * the product's does (`product.md` § Steer or queue): its text, and its
 * files as capsules a paste brings back, the caret at its end; a draft it
 * holds stays as it is.
 */
const recall: RecallComposer = {
  takeBack(message) {
    if (!isEmptyMessage(draft.value, carried.value.length)) {
      return false
    }
    const pasted = pasteMessage(message, null, paste)
    carried.value = pasted.capsules.map((capsule) => capsule.id)
    draft.value = pasted.markdown
    shown.value += 1
    composer.value?.focusDraft()
    const missing = missingFilesToast(pasted.missing)
    if (missing) {
      showToast({ ...missing, tone: 'danger' })
    }
    return true
  },
  sendAgain(message, way) {
    const pasted = pasteMessage(message, null, paste)
    const sent = pasted.capsules.flatMap((capsule) => attached.value.filter((item) => item.id === capsule.id))
    handOn(pasted.markdown, sent, way)
  },
}
let detach: (() => void) | undefined
onMounted(() => {
  detach = props.flow?.attachComposer(recall)
})

/**
 * Uploads a file, or attaches the stored file a pasted message names through
 * the stand-in's claim, uploading the picture the copy carried when it cannot
 * use the file, as the product's does. Answers the transfer, which a message
 * sent meanwhile waits for.
 */
function startUpload(id: string): Promise<void> {
  const item = attached.value.find((each): each is ComposerFileAttachment => each.id === id && isComposerFile(each))
  if (!item || (!picked.has(id) && !pastedClaims.has(id))) {
    return Promise.resolve()
  }
  // The stand-in's upload never fails; were it to, the file would keep its capsule with Retry, as the product's does.
  // The item is held, not looked up, so a sent message's capsule goes on showing its upload.
  const transfer = uploads.start(id, async (signal, report) => {
    const answer = await transferOf(id, item, signal, report)
    answers.set(id, answer)
    item.snippet = answer.snippet
  }, (update) => applyAttachmentUpdate(item, attachingUpdate(update, picked.has(id))))
    .then(() => {}, () => applyAttachmentUpdate(item, { phase: 'failed' }))
  transfers.set(id, transfer)
  void transfer.then(() => {
    if (transfers.get(id) === transfer) {
      transfers.delete(id)
    }
  })
  return transfer
}

/** What a file's transfer answers: its upload, or the claim of the stored file a paste names, else its picture's upload. */
async function transferOf(
  id: string,
  item: ComposerFileAttachment,
  signal: AbortSignal,
  report: (fraction: number) => void,
): Promise<UploadedFile> {
  const claim = pastedClaims.get(id)
  if (claim) {
    const answer = await host.claim(claim.file, signal)
    if (answer) {
      // The file's size, for the record a sent message keeps.
      picked.set(id, new File([], claim.file.name, { type: claim.file.mediaType }))
      return answer
    }
    if (!claim.picture) {
      throw new Error(`${claim.file.name} is not among this account's files.`)
    }
    pastedClaims.delete(id)
    picked.set(id, claim.picture)
    item.src = filePreviewUrl(claim.picture) ?? item.src
  }
  return upload(picked.get(id)!, { signal, progress: report })
}

/** Takes files and puts their capsules in the message, where the composer said they would land. */
function addFiles(files: File[]) {
  composer.value?.insertCapsules(takeFiles(files).map(composerCapsule))
}

/**
 * How a message pasted into the composer brings its files back: a blob the
 * gallery holds as an upload without its bytes, a device file by its path,
 * and the picture a copy carried as a pasted file.
 */
const paste: PasteHost = {
  claim: (file, picture) => {
    const item = composerAttachment({ name: file.name, src: host.picture(file.sha256, file.mediaType), phase: 'attaching' })
    attached.value.push(item)
    pastedClaims.set(item.id, { file, picture })
    const kept = startUpload(item.id).then(() => answers.has(item.id) || picked.has(item.id))
    return { capsule: composerCapsule(item), kept }
  },
  remote: (file) => {
    const item = composerRemoteAttachment({ host: file.host, path: file.path })
    attached.value.push(item)
    return composerCapsule(item)
  },
}

/** Takes files into the composer, each starting its upload, and answers the ones it took. */
function takeFiles(files: File[]): ComposerAttachment[] {
  const taken: ComposerAttachment[] = []
  for (const file of files) {
    if (attachmentFileError(file)) {
      continue
    }
    const item = composerAttachmentFromFile(file)
    picked.set(item.id, file)
    attached.value.push(item)
    const held = attached.value.find((each) => each.id === item.id)!
    taken.push(held)
    startUpload(item.id)
  }
  return taken
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
  const item = composerRemoteAttachment(file)
  attached.value.push(item)
  composer.value?.insertCapsules([composerCapsule(item)])
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
  detach?.()
  window.clearTimeout(reloadTimer)
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
    :claim="host.claim"
    :paste="paste"
    :providers="props.providers ?? demoProviders"
    :models="props.models ? withContextLimits(props.models) : composerModels"
    :can-configure="canConfigure !== false"
    :archived="archived"
    :hold="hold"
    :model-settings="settings"
    :usage="props.usage ?? usageAt(0.62)"
    :instructions="demoInstructions"
    :replaced="replacedVersion && { markdown: replacedVersion.text, fileNames: replacedVersion.files.map((file) => file.name) }"
    :draft-shown="shown"
    :drafts="draftRows"
    v-model:drafts-open="draftsOpen"
    @save-draft="saveDraft"
    @edit-draft="editDraft"
    @send-draft="sendDraft"
    @delete-draft="deleteDraft"
    :plugins-changed="pluginsChanged"
    :reloading="reloading"
    :offline-host="offlineHost"
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
