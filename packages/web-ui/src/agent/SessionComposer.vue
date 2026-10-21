<script setup lang="ts">
import { t } from './locales/text'
import { t as shared } from '../locales/text'
import { computed, inject, ref, useSlots, watch } from 'vue'
import { isFocusedElementEditable, useElementSize } from '@vueuse/core'
import type { ContextUsage, InstructionEntry } from '@demicodes/protocol'
import { ArrowUp, File as FileIcon, HardDrive, NotebookPen, Paperclip, Pencil, RotateCcw, Save, Square, X } from '@lucide/vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '../overlay/appOverlay'
import {
  attachmentSendBlockReason,
  isComposerFile,
  type ClaimFile,
  type ComposerAttachment,
  type UploadFile,
} from './message-input/attachments'
import { missingFilesToast, pasteMessage, type CopiedMessage, type PasteHost } from './message-input/copied-message'
import { useMessageEditComposer } from './message-input/useMessageEditComposer'
import { draftPreview } from './message-input/draft-preview'
import { editHasContent, editKey, useEditLastMessage, type MessageEditState } from './message-editing'
import { composerCapsule, composerTransfer, provideTransfers, type MessageCapsule } from './message-editor/capsules'
import { parseUserMarkdown, serializeUserMarkdown } from '../markdown/user-markdown'
import MessageEditor from './message-editor/MessageEditor.vue'
import { showToast } from '../infra/toast'
import ComposerShell from './ComposerShell.vue'
import SavedEntry from './SavedEntry.vue'
import SavedList from './SavedList.vue'
import { asksSaveForLater, saveForLaterKeys, type SavedRow } from './saved'
import ContextUsageIndicator from './ContextUsageIndicator.vue'
import { compactAfterTurn, type UsageLimits } from './context-usage'
import type { CostShown } from './context-cost'
import ModelSelector from './ModelSelector.vue'
import WorkflowMenu from './WorkflowMenu.vue'
import type { WorkflowChoice, WorkflowList } from './workflow-menu'
import { composerModel, type ModelSettings, type ModelSettingsChange } from './model-selection'
import type { ContextLimitChange } from './context-limit'
import SessionNoticeBar from './SessionNoticeBar.vue'
import ReplacedDraftNotice from './ReplacedDraftNotice.vue'
import PluginsChangedNotice from './PluginsChangedNotice.vue'
import HostOfflineNotice from './HostOfflineNotice.vue'
import type { OfflineHost } from './offline-host'
import { dockCardsKey } from './dock-cards'
import type { HostChoice } from '../hosts/types'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import { ICON_PX } from '../ui/icon-metrics'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { Action } from '../ui/operations'
import type { PlaceholderText } from '../ui/ui-text'
import {
  DEFAULT_SEND_WHILE_RUNNING,
  sendWayLabel,
  chooseSendWay,
  clickAsksOtherWay,
  otherWay,
  otherWayKeys,
  useSendOnceLoaded,
  type SendWay,
  type SendWhileRunning,
} from './send-way'

const props = withDefaults(
  defineProps<{
    placeholder?: PlaceholderText
    /**
     * A turn runs, or the message that starts one is on its way. Stop shows
     * from the send to the turn's end, and a message is queued meanwhile.
     */
    running?: boolean
    compacting?: boolean
    /**
     * What Enter does with a message while the agent works; ⌘/Ctrl+Enter does
     * the other.
     */
    sendWhileRunning?: SendWhileRunning
    disabled?: boolean
    attachments?: ComposerAttachment[]
    messageEdit?: MessageEditState | null
    /**
     * Uploads a file the edit composer adds, as the main composer's files are
     * uploaded.
     */
    upload: UploadFile
    /**
     * Makes an upload of a file the user's blobs hold, for a message pasted
     * into the edit composer.
     */
    claim: ClaimFile
    /**
     * Takes the files of a message pasted into the draft. The host carries the
     * draft's files.
     */
    paste: PasteHost
    /**
     * The conversation has a host with files: the menu offers a remote file
     * beside local ones.
     */
    remoteFiles?: boolean
    focused?: boolean
    /**
     * The draft takes the focus as the composer shows, so a new or opened
     * conversation is typed into at once. Not on a phone, where the keyboard
     * waits for a tap (`useAutofocus`), and not while another field holds the
     * focus.
     */
    focusOnShow?: boolean
    attachOpen?: boolean
    dropping?: boolean
    modelLoad?: 'loading' | 'ready' | 'failed'
    providers: ProviderInfo[]
    models: Record<string, ModelInfo[]>
    /** The conversation's model settings; none while nothing is chosen. */
    modelSettings?: ModelSettings | null
    /** How full the next request is, as the backend reports it. */
    usage?: ContextUsage | null
    /**
     * The unfinished turn failed for its size, so the context card offers
     * Compact at any share.
     */
    sizeRefused?: boolean
    /**
     * What the newest instructions block holds, which the context card lists.
     */
    instructions?: readonly InstructionEntry[]
    /**
     * The usage limits of the account the model runs on, which the context card
     * shows; none on an API key.
     */
    limits?: UsageLimits | null
    /**
     * What the conversation cost, from its agents' metadata, with the calls
     * the host reads while the context card is open.
     */
    cost?: CostShown | null
    /**
     * The names of the profiles the conversation's workflow offers now, which
     * the cost's Profiles part shows.
     */
    costProfiles?: readonly string[]
    /**
     * When no model can send, show Configure Models. Hide the action if the
     * user cannot open that page.
     */
    canConfigure?: boolean
    /** Replaces the input with the archive bar. */
    archived?: boolean
    /**
     * Why nothing can be sent for now, such as `Cloud is resetting.` The input
     * gives way to that line and returns, draft kept, when the caller clears
     * it.
     */
    hold?: string | null
    /**
     * The version of the draft a later save replaced, which the composer offers
     * to restore. It holds the Markdown and the names of the files.
     */
    replaced?: { markdown: string; fileNames: readonly string[] } | null
    /**
     * The conversation's primary Host is a paired device that is offline. The
     * composer says how to start its runner again, and offers a move to Run
     * On's online Hosts.
     */
    offlineHost?: OfflineHost | null
    /**
     * The conversation runs with commands of plugins since turned on or off:
     * offers a reload.
     */
    pluginsChanged?: boolean
    /** Stops the turn; Stop works until the session took it. */
    onStop?: Action
    /**
     * The conversation's ID, which names the target of Stop (`operations.ts`),
     * so Stop shows its own conversation's work only.
     */
    conversationId?: string
    /**
     * Opens a saved message in the composer's edit mode, as a `messageEdit`
     * of a saved message; the message stays in the list, in its place.
     */
    onEditSaved?: Action<[id: string]>
    /** Sends a saved message as the composer would, the way it chooses now. */
    onSendSaved?: Action<[id: string, way: SendWay]>
    /** Removes a saved message at once; the caller offers the undo. */
    onDeleteSaved?: Action<[id: string]>
    /** Saves again a message the server did not save. */
    onRetrySaved?: Action<[id: string]>
    /** The reload the composer offered is under way. */
    reloading?: boolean
    /**
     * Counts the drafts shown from outside, another page's or a restored one.
     * When it changes, the editor shows the draft and its files anew, even when
     * the text is the same.
     */
    draftShown?: number
    /**
     * The conversation's Workflow menu choice and the project's workflows. None
     * where a conversation has no choice yet, as before its first send.
     */
    workflow?: { choice: WorkflowChoice; list: WorkflowList } | null
    /**
     * The conversation's messages saved for later, newest first. The entry
     * under the composer counts them and opens their list.
     */
    saved?: SavedRow[]
  }>(),
  {
    attachments: () => [],
    saved: () => [],
    canConfigure: true,
    sendWhileRunning: DEFAULT_SEND_WHILE_RUNNING,
  },
)
const draft = defineModel<string>('draft', { default: '' })
/** The list of messages saved for later stands open under the composer. */
const savedOpen = defineModel<boolean>('savedOpen', { default: false })
const emit = defineEmits<{
  retryModels: []
  /**
   * The draft goes: at once while nothing runs, otherwise as a steer or to the
   * queue.
   */
  submit: [way: SendWay]
  /**
   * Keep the message, its files included, for later, and empty the composer.
   */
  saveForLater: []
  compact: []
  /** Opens where an instructions entry of the context card is written. */
  openInstruction: [entry: InstructionEntry]
  /** The context card's Refresh usage button asks the vendor for the usage limits again. */
  refreshLimits: []
  /**
   * Files to attach; their capsules land where they were dropped, pasted or
   * picked.
   */
  addFiles: [files: File[]]
  /**
   * Open the host's file browser; the caller attaches what it returns, and its
   * capsule lands at the cursor.
   */
  attachRemote: []
  /** The files the message now carries, in the order of their capsules. */
  arrangeAttachments: [ids: string[]]
  /** Try a failed upload again. */
  retryAttachment: [id: string]
  /** One change of the model settings, naming only the parts it changes. */
  changeModel: [change: ModelSettingsChange]
  /** The conversation's new Workflow menu choice. */
  changeWorkflow: [choice: WorkflowChoice]
  /**
   * The user's context limit on a model, for all their conversations with it.
   */
  changeContextLimit: [change: ContextLimitChange]
  configure: []
  restore: []
  'update:messageEdit': [state: MessageEditState | null]
  /**
   * Sends the edit shown: a sent message's as its edit, a saved message's the
   * way the composer sends a message now.
   */
  submitEdit: [way: SendWay]
  /**
   * Writes the saved message's edit back to the message, in its place, and
   * ends the edit.
   */
  saveEdit: []
  /** Bring back the replaced version, in exchange for the draft. */
  restoreReplaced: []
  dismissReplaced: []
  /** Open the conversation again with the plugins the user has on. */
  reloadPlugins: []
  /**
   * A Host the offline primary Host's card chose to move the conversation to.
   */
  moveHost: [host: HostChoice]
}>()
/**
 * The edit the composer shows. It is none while Regenerate sends one, which
 * never opens the editor. The draft stays, and waits for that edit to be
 * answered.
 */
const regenerating = computed(() => props.messageEdit?.phase === 'regenerating')
const shownEdit = computed(() => regenerating.value ? null : props.messageEdit ?? null)
/** The edit shown is of a message saved for later. */
const savedEdit = computed(() => shownEdit.value?.kind === 'saved')
const edit = useMessageEditComposer({
  state: () => shownEdit.value,
  update: (state) => emit('update:messageEdit', state),
  upload: (file, options) => props.upload(file, options),
  claim: (file, signal) => props.claim(file, signal),
})
/**
 * Up Arrow in the empty draft opens the editor on the last message, where the
 * page offers editing it.
 */
const editLastMessage = useEditLastMessage()
const editLast = computed(() => props.disabled ? undefined : editLastMessage())
/**
 * Where a dock stands the composer's cards, above its chips. None outside a
 * dock, where the cards stand over the input.
 */
const dockCards = inject(dockCardsKey, () => ref<HTMLElement>(), true)
/** Read once, as the composer shows: the focus it finds then decides. */
const takesFocus = props.focusOnShow && !isFocusedElementEditable()
/** The editor shown: the edit's, or the draft's. */
const editor = ref<InstanceType<typeof MessageEditor>>()
const focused = ref(false)
// An editor taken away while it has the focus tells no blur; the one in its
// place starts unfocused.
watch(() => shownEdit.value && editKey(shownEdit.value), () => {
  focused.value = false
})
/**
 * The user sent the edit shown. Its editor takes no typing while it is sent,
 * and gives way to the draft's once the edit ends. The draft's editor then
 * takes the focus, so the next message is typed at once, as after a send,
 * unless the user has put the focus elsewhere meanwhile.
 */
let editSent = false
watch(editor, (current) => {
  if (!current || shownEdit.value || !editSent) {
    return
  }
  editSent = false
  if (document.activeElement === document.body) {
    current.focus()
  }
})
/** The message holds more than one line, so the composer grows to hold it. */
const multiline = ref(false)
const fileInput = ref<HTMLInputElement>()
/**
 * The send button, which says why it cannot send when the message is given to
 * it anyway.
 */
const sendButton = ref<InstanceType<typeof IconButton>>()
/** What the composer carries, as the editor builds its document from it. */
const carried = computed(() => props.attachments.map(composerCapsule))
/**
 * The files the message has, in the order of their capsules, as the document
 * says.
 *
 * It starts as the editor builds it, with the files the draft's marks stand
 * for, and not those the composer still holds for an undo. From then on the
 * editor tells every change.
 */
const capsules = ref<MessageCapsule[]>(
  serializeUserMarkdown<MessageCapsule>(parseUserMarkdown(draft.value, carried.value)).attachments,
)
/**
 * Their transfers, and those of the files an edit adds, which the capsules in
 * the editor read.
 */
provideTransfers({
  transfer: (id) => {
    const item = props.attachments.find((each) => each.id === id)
    return item ? composerTransfer(item) : edit.transfers.transfer(id)
  },
  carries: (id) => props.attachments.some((each) => each.id === id) || edit.transfers.carries(id),
  retry: (id) => {
    if (props.attachments.some((each) => each.id === id)) {
      emit('retryAttachment', id)
    } else {
      edit.transfers.retry(id)
    }
  },
  current: (id) => {
    const item = props.attachments.find((each) => each.id === id)
    return item ? composerCapsule(item) : edit.transfers.current(id)
  },
})
/**
 * The files of the message, not the ones the composer still holds for an undo.
 */
const carrying = computed(() => {
  const ids = new Set(capsules.value.map((capsule) => capsule.id))
  return props.attachments.filter((item) => ids.has(item.id))
})
const slots = useSlots()
/**
 * What the page sends with the message besides its draft, such as a batch of
 * comments, which its `context` slot shows inside the box.
 *
 * While it is there, the box takes its multi-line layout. Send sends it with
 * the draft, or alone when the draft is empty. It is read at each render and
 * each send, as a slot is.
 */
const carriesContext = () => slots.context !== undefined
const hasDraft = computed(
  () => shownEdit.value ? editHasContent(shownEdit.value)
    : !!draft.value.trim() || !!capsules.value.length,
)
/**
 * Save for Later keeps a message that holds something; not an edit of a sent
 * one.
 */
const canSaveForLater = computed(() => hasDraft.value && !shownEdit.value && !props.disabled)
/**
 * Save writes a saved message's edit back once it holds something and its
 * files are all in.
 */
const canSaveEdit = computed(
  () => savedEdit.value && edit.editable.value && hasDraft.value && !edit.sendBlockReason.value,
)
/**
 * The composer's width under which the entry keeps its mark and count. The
 * model's chip and the context ring need the rest of the row.
 */
const COMPACT_ENTRY_BELOW_PX = 380
const composerRoot = ref<HTMLElement | null>(null)
const { width: composerWidth } = useElementSize(composerRoot)
const compactEntry = computed(() => composerWidth.value > 0 && composerWidth.value < COMPACT_ENTRY_BELOW_PX)
/**
 * Edit opens the message in the composer, so the list gives its room back;
 * Send and Delete keep it open for the next.
 */
function editSaved(id: string): unknown {
  savedOpen.value = false
  return props.onEditSaved?.(id)
}
/**
 * Send goes the way the composer's send button would now: at once, or steering
 * or queueing while the agent works.
 */
function sendSaved(id: string): unknown {
  return props.onSendSaved?.(id, chooseSendWay(working.value, props.sendWhileRunning, false))
}
// The list closes with its last message, so the next one does not open it
// unasked.
watch(() => props.saved.length, (count) => {
  if (count === 0) {
    savedOpen.value = false
  }
})

/**
 * ⌘S (Ctrl+S) saves the message for later, or a saved message's edit as Save
 * does, wherever the focus is in the composer, never the page.
 */
function saveKey(event: KeyboardEvent) {
  if (!asksSaveForLater(event)) {
    return
  }
  event.preventDefault()
  if (savedEdit.value) {
    saveEdit()
  } else if (canSaveForLater.value) {
    emit('saveForLater')
  }
}

const modelState = computed(() =>
  composerModel(
    props.providers,
    props.models,
    props.modelSettings?.providerId,
    props.modelSettings?.modelId,
  ),
)
/**
 * The models are still loading. The page cannot tell yet whether the message
 * can go.
 */
const modelsLoading = computed(() => props.modelLoad === 'loading' && modelState.value.kind !== 'ready')
/**
 * The phases of the draft's uploads that keep it from going: a failed one, or a
 * file that is gone. A message whose files are still uploading goes, and waits
 * for them.
 */
const failedUploads = computed(() => carrying.value
  .filter(isComposerFile)
  .flatMap((item) => item.phase === 'failed' || item.phase === 'missing' ? [item.phase] : []))
const sendDisabled = computed(
  () =>
    props.disabled ||
    regenerating.value ||
    modelState.value.kind !== 'ready' ||
    (shownEdit.value
      ? shownEdit.value.phase === 'sending' || !!edit.sendBlockReason.value
      : failedUploads.value.length > 0),
)
const sendBlockReason = computed(() => {
  if (modelState.value.kind === 'unavailable') {
    return t('composer.modelUnavailable')
  }
  if (modelsLoading.value) {
    return t('composer.modelsLoading')
  }
  if (props.disabled) {
    return undefined
  }
  return shownEdit.value
    ? edit.sendBlockReason.value
    : attachmentSendBlockReason(failedUploads.value)
})
// The composer shows no failure text of its own. Failures show elsewhere:
//
// - A file an edit could not take or upload is a toast.
// - A failed upload is its capsule's Retry.
// - A refused edit is the product's toast.
watch(edit.attachmentError, (message) => {
  if (message) {
    showToast({ title: t('composer.couldntAttach'), message, tone: 'danger' })
  }
})
/**
 * The replaced version as the offer shows it; none while there is none, or
 * while a message is edited.
 */
const replacedPreview = computed(() =>
  props.replaced && !shownEdit.value
    ? draftPreview(props.replaced.markdown, props.replaced.fileNames)
    : null,
)
/**
 * Why Compact cannot run now apart from how full the context is: the session
 * takes it only while idle.
 */
const compactUnavailable = computed(() => {
  if (shownEdit.value) {
    return t('composer.afterEdit')
  }
  if (props.running) {
    return compactAfterTurn()
  }
  return null
})
/**
 * The agent works. A message steers it or waits in the queue, one or the other.
 */
const working = computed(() => props.running || props.compacting)
const submitLabel = computed(() => shownEdit.value && !savedEdit.value
  ? shownEdit.value.phase === 'uncertain' ? shared('common.retry') : t('composer.saveAndResend')
  : working.value ? sendWayLabel(props.sendWhileRunning) : t('composer.send'),
)
/**
 * The send button's tip: while the agent works, also the key that sends the
 * other way.
 */
const submitTip = computed(() => (!shownEdit.value || savedEdit.value) && working.value
  ? t('composer.sendOtherWay', {
      way: submitLabel.value,
      keys: otherWayKeys(),
      other: sendWayLabel(otherWay(props.sendWhileRunning)).toLowerCase(),
    })
  : submitLabel.value,
)

/**
 * Enter pressed on the draft while the models load: the message goes once they
 * have, if it still holds one.
 */
const waitingSend = useSendOnceLoaded({
  blocked: () => sendDisabled.value,
  loading: () => modelsLoading.value,
  send: (asksOtherWay) => {
    if ((hasDraft.value || carriesContext()) && !shownEdit.value) {
      emit('submit', chooseSendWay(working.value, props.sendWhileRunning, asksOtherWay))
    }
  },
})

/**
 * Sends the message; `asksOtherWay` for ⌘/Ctrl+Enter outside a code block, or a
 * ⌘/Ctrl-click on the send button.
 */
function submit(asksOtherWay: boolean) {
  const sendsContext = !shownEdit.value && carriesContext()
  if (sendDisabled.value) {
    if ((hasDraft.value || sendsContext) && !shownEdit.value) {
      waitingSend.keep(asksOtherWay)
    }
    // Enter on a message that cannot go: the send button says why, unpointed.
    sendButton.value?.showReason()
    return
  }
  if (!hasDraft.value && !sendsContext) {
    return
  }
  const way = chooseSendWay(working.value, props.sendWhileRunning, asksOtherWay)
  if (shownEdit.value) {
    editSent = true
    emit('submitEdit', way)
    return
  }
  emit('submit', way)
  // The next message is typed at once, wherever the focus was when it was sent.
  editor.value?.focus()
}

function saveEdit() {
  if (canSaveEdit.value) {
    emit('saveEdit')
  }
}

function pickFiles(close: () => void) {
  close()
  editor.value?.placeNextFiles()
  fileInput.value?.click()
}

function fileChange(event: Event) {
  const input = event.target as HTMLInputElement
  addFiles([...(input.files ?? [])])
  input.value = ''
  // The menu that opened the chooser took the focus with it: the message gets
  // it back, to go on typing.
  editor.value?.focus()
}

function dropFiles(files: File[], event: DragEvent) {
  editor.value?.placeNextFiles(event)
  addFiles(files)
}

function attachRemote() {
  editor.value?.placeNextFiles()
  emit('attachRemote')
}

function addFiles(files: File[]): void {
  if (!shownEdit.value) {
    emit('addFiles', files)
    return
  }
  // An edit uploads its files itself; their capsules go in where they were told
  // to land.
  insertCapsules(edit.addFiles(files))
}

/**
 * Puts files the host took into the message, where the composer said they would
 * land.
 */
function insertCapsules(added: readonly MessageCapsule[]): void {
  editor.value?.insertCapsules(added)
}

/**
 * A copied message pasted.
 *
 * The message goes in where it was pasted at once, text and capsules. Each
 * stored file attaches beside it as an upload does, so the message can be sent
 * meanwhile. A file the paste could not add stands as its name where its
 * capsule stood, and a toast names it once the paste's files have all answered.
 */
function pasteCopied(message: CopiedMessage, image: File | null): void {
  // The editor the message was pasted into, which an edit opened or closed
  // meanwhile does not change.
  const target = editor.value
  const pasted = pasteMessage(message, image, shownEdit.value ? edit.paste : props.paste)
  target?.insertMessage(pasted.markdown, pasted.capsules)
  void Promise.all(pasted.attaching.map(async ({ id, name, kept }) =>
    // A file of a message sent meanwhile is no longer the composer's: its
    // bubble tells it.
    !await kept && target?.replaceCapsule(id, name) ? [name] : []))
    .then((names) => missingFiles([...pasted.missing, ...names.flat()]))
}

/** Tells which pasted files could not be added, their names pasted instead. */
function missingFiles(names: readonly string[]): void {
  const toast = missingFilesToast(names)
  if (toast) {
    showToast({ ...toast, tone: 'danger' })
  }
}

defineExpose({
  /**
   * Files the host has taken: their capsules go into the message where they
   * were told to land.
   */
  insertCapsules,
  /** Files are on their way: the next capsules land at the cursor. */
  placeNextFiles(event?: DragEvent): void {
    editor.value?.placeNextFiles(event)
  },
  /**
   * The user asked for this draft again, as New does on the draft it shows. The
   * message takes the focus, except on a phone (`useAutofocus`).
   */
  focusDraft(): void {
    editor.value?.takeFocus()
  },
})

function changeDraft(markdown: string, attachments: MessageCapsule[]): void {
  draft.value = markdown
  const had = capsules.value.map((capsule) => capsule.id).join('\n')
  capsules.value = attachments
  const ids = attachments.map((capsule) => capsule.id)
  // The host hears which files the message has only when that changes; its text
  // it hears every time.
  if (ids.join('\n') !== had) {
    emit('arrangeAttachments', ids)
  }
}
</script>

<template>
  <Transition name="composer-archive" mode="out-in">
    <SessionNoticeBar
      v-if="archived"
      key="archived"
      :label="t('session.archived')"
      :action="t('session.restore')"
      @action="emit('restore')"
    />
    <SessionNoticeBar
      v-else-if="hold"
      key="hold"
      :label="hold"
      busy
    />
    <SessionNoticeBar
      v-else-if="
        modelState.kind === 'none' && (!modelLoad || modelLoad === 'ready')
      "
      key="none"
      :label="t('composer.noModels')"
      :action="
        canConfigure !== false ? t('composer.configureModels') : undefined
      "
      @action="emit('configure')"
    />
    <div v-else key="composer" ref="composerRoot" class="w-full" @keydown="saveKey">
      <input
        ref="fileInput"
        type="file"
        class="hidden"
        multiple
        @change="fileChange"
      />
      <!--
        The cards stand above the dock's chips, which sit directly on the input.
        The dock's place for them exists once the dock is mounted. Until then,
        and outside a dock, they stand here.
      -->
      <Teleport :to="dockCards" :disabled="!dockCards">
        <HostOfflineNotice
          v-if="offlineHost"
          class="mb-2"
          v-bind="offlineHost"
          @move="emit('moveHost', $event)"
        />
        <PluginsChangedNotice
          v-if="pluginsChanged"
          class="mb-2"
          :reloading="reloading"
          @reload="emit('reloadPlugins')"
        />
        <ReplacedDraftNotice
          v-if="replacedPreview !== null"
          class="mb-2"
          :preview="replacedPreview"
          @restore="emit('restoreReplaced')"
          @dismiss="emit('dismissReplaced')"
        />
      </Teleport>
      <ComposerShell
        :focused="focused || props.focused"
        :expanded="multiline || !!shownEdit || carriesContext()"
        :captioned="!!shownEdit"
        :dropping="dropping"
        @drop-files="dropFiles"
      >
        <!--
          An edit says what it edits where the draft shows what it carries,
          marked as what opened it: the Saved entry's mark, or Edit's pencil.
        -->
        <template v-if="shownEdit || carriesContext()" #context>
          <span v-if="shownEdit" class="inline-flex select-none items-center text-fg-subtle">
            <component :is="savedEdit ? NotebookPen : Pencil" :size="ICON_PX.in28" class="mr-1 shrink-0" />
            {{ savedEdit ? t('composer.editingSaved') : t('composer.editingMessage') }}
          </span>
          <slot v-else name="context" />
        </template>
        <template #editor="{ line }">
          <MessageEditor
            v-if="shownEdit"
            :key="editKey(shownEdit)"
            ref="editor"
            v-model:multiline="multiline"
            cancelable
            autofocus
            asked
            :line-width="line"
            :disabled="!edit.editable.value"
            :markdown="edit.markdown.value"
            :attachments="edit.capsules.value"
            :placeholder="placeholder ?? t('composer.placeholder')"
            :label="t('composer.message')"
            @change="edit.change"
            @submit="submit"
            @cancel="edit.cancel"
            @files="addFiles"
            @message="pasteCopied"
            @focus="focused = true"
            @blur="focused = false"
          />
          <MessageEditor
            v-else
            ref="editor"
            v-model:multiline="multiline"
            :line-width="line"
            :markdown="draft"
            :attachments="carried"
            :shown-from-outside="draftShown"
            :autofocus="takesFocus"
            :placeholder="placeholder ?? t('composer.placeholder')"
            :edit-last="editLast"
            :label="t('composer.message')"
            @change="changeDraft"
            @submit="submit"
            @files="addFiles"
            @message="pasteCopied"
            @focus="focused = true"
            @blur="focused = false"
          />
        </template>
        <template #attach>
          <!--
            The menu opens above the composer, over the conversation it sits
            under. Pinned open in a gallery well, which has its room below, it
            opens below.
          -->
          <Dropdown
            v-if="!shownEdit || edit.editable.value"
            :overlay-store="appOverlayStore"
            :side="attachOpen ? 'bottom' : 'top'"
            v-bind="attachOpen ? { open: true } : {}"
          >
            <template #trigger="{ isOpen }">
              <!--
                A press on the paperclip takes no focus, as on the send button.
                The message keeps it until the menu takes it, and gets it back
                when the menu closes.
              -->
              <Tooltip :content="t('composer.attach')">
                <IconButton
                  :icon="Paperclip"
                  variant="ghost"
                  circle
                  :pressed="isOpen"
                  :aria-label="t('composer.attach')"
                  @mousedown.prevent
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  :icon="FileIcon"
                  :label="remoteFiles ? t('composer.attachLocalFiles') : t('composer.attachFiles')"
                  @select="pickFiles(close)"
                />
                <MenuItem
                  v-if="remoteFiles && !shownEdit"
                  :icon="HardDrive"
                  :label="t('composer.attachRemoteFile')"
                  @select="attachRemote"
                />
              </Menu>
            </template>
          </Dropdown>
        </template>
        <template v-if="(workflow || saved.length) && (!shownEdit || savedEdit)" #below-start>
          <div class="flex items-center gap-1">
            <WorkflowMenu
              v-if="workflow"
              :choice="workflow.choice"
              :list="workflow.list"
              :compact="compactEntry"
              @change="emit('changeWorkflow', $event)"
            />
            <SavedEntry
              v-if="saved.length"
              :count="saved.length"
              :open="savedOpen"
              :compact="compactEntry"
              @toggle="savedOpen = !savedOpen"
            />
          </div>
        </template>
        <template v-if="savedOpen && saved.length && (!shownEdit || savedEdit)" #list>
          <SavedList
            :rows="saved"
            @edit="editSaved"
            @send="sendSaved"
            @delete="onDeleteSaved?.($event)"
            @retry="onRetrySaved?.($event)"
          />
        </template>
        <template #below>
          <div class="min-w-0" :inert="!!shownEdit && !edit.editable.value">
            <ModelSelector
              :load="modelLoad"
              @retry="emit('retryModels')"
              :providers="providers"
              :models="models"
              :settings="modelSettings"
              @change="emit('changeModel', $event)"
              @context-limit="emit('changeContextLimit', $event)"
            />
          </div>
          <ContextUsageIndicator
            :usage="usage"
            :size-refused="sizeRefused"
            :is-compacting="compacting"
            :unavailable-reason="compactUnavailable"
            :instructions="instructions"
            :limits="limits"
            :cost="cost"
            :cost-profiles="costProfiles"
            @compact="emit('compact')"
            @open-instruction="emit('openInstruction', $event)"
            @refresh-limits="emit('refreshLimits')"
          />
        </template>

        <template #actions>
          <Tooltip v-if="shownEdit" :content="t('composer.cancelEdit')">
            <IconButton
              :icon="X"
              variant="ghost"
              circle
              :aria-label="t('composer.cancelEdit')"
              :disabled="!edit.editable.value"
              :tabindex="edit.editable.value ? 0 : -1"
              @click="edit.cancel"
              @keydown.enter.space.prevent="edit.cancel"
            />
          </Tooltip>
          <Tooltip v-if="savedEdit" :content="t('composer.saveEditKeys', { keys: saveForLaterKeys() })">
            <IconButton
              :icon="Save"
              variant="ghost"
              circle
              :aria-label="t('composer.saveEdit')"
              :disabled="!canSaveEdit"
              @mousedown.prevent
              @click="saveEdit"
            />
          </Tooltip>
          <Tooltip v-if="canSaveForLater" :content="t('composer.saveForLaterKeys', { keys: saveForLaterKeys() })">
            <IconButton
              :icon="NotebookPen"
              variant="ghost"
              circle
              :aria-label="t('composer.saveForLater')"
              @mousedown.prevent
              @click="emit('saveForLater')"
            />
          </Tooltip>
          <!--
            Stop stays for the whole turn, beside the send button while the
            message holds text.
          -->
          <Tooltip v-if="(running || compacting) && !shownEdit" :content="t('composer.stop')">
            <IconButton
              :icon="Square"
              variant="ghost"
              circle
              :aria-label="t('composer.stop')"
              data-composer-stop
              :operation="conversationId ? { action: 'stop', target: `conversation:${conversationId}` } : undefined"
              @mousedown.prevent
              @click="onStop?.()"
            />
          </Tooltip>
          <!--
            A press on the send button takes no focus: the message keeps it. A
            ⌘/Ctrl-click sends the other way while the agent works.
          -->
          <Tooltip
            v-if="hasDraft || (!shownEdit && carriesContext())"
            :content="submitTip"
            :disabled="sendDisabled"
          >
            <IconButton
              ref="sendButton"
              :icon="shownEdit?.phase === 'uncertain' ? RotateCcw : ArrowUp"
              variant="accent"
              circle
              :disabled="sendDisabled"
              :loading="shownEdit?.phase === 'sending'"
              :disabled-reason="sendBlockReason"
              :aria-label="submitLabel"
              @mousedown.prevent
              @click="submit(clickAsksOtherWay($event))"
            />
          </Tooltip>
          <IconButton
            v-else-if="!running && !compacting"
            :icon="ArrowUp"
            variant="ghost"
            circle
            disabled
            :aria-label="t('composer.send')"
          />
        </template>
      </ComposerShell>
    </div>
  </Transition>
</template>

<style scoped>
.composer-archive-enter-active,
.composer-archive-leave-active {
  transition:
    opacity 140ms ease,
    transform 140ms ease;
}

.composer-archive-enter-from,
.composer-archive-leave-to {
  opacity: 0;
  transform: translateY(6px);
}

@media (prefers-reduced-motion: reduce) {
  .composer-archive-enter-active,
  .composer-archive-leave-active {
    transition: none;
  }
}
</style>
