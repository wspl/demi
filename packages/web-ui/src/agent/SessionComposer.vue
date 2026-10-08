<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { isFocusedElementEditable } from '@vueuse/core'
import type { ContextUsage } from '@demicodes/protocol'
import { ArrowUp, File as FileIcon, HardDrive, Plus, RotateCcw, Square, X } from '@lucide/vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '../overlay/appOverlay'
import {
  attachmentsReady,
  attachmentSendBlockReason,
  isComposerFile,
  type ComposerAttachment,
  type UploadFile,
} from './message-input/attachments'
import { useMessageEditComposer } from './message-input/useMessageEditComposer'
import { draftPreview } from './message-input/draft-preview'
import { editHasContent, useEditLastMessage, type MessageEditState } from './message-editing'
import { composerCapsule, composerTransfer, provideTransfers, type MessageCapsule } from './message-editor/capsules'
import { parseUserMarkdown, serializeUserMarkdown } from '../markdown/user-markdown'
import MessageEditor from './message-editor/MessageEditor.vue'
import { showToast } from '../infra/toast'
import ComposerShell from './ComposerShell.vue'
import ContextUsageIndicator from './ContextUsageIndicator.vue'
import ModelSelector from './ModelSelector.vue'
import { composerModel, type ModelSettings, type ModelSettingsChange } from './model-selection'
import type { ContextLimitChange } from './context-limit'
import SessionNoticeBar from './SessionNoticeBar.vue'
import ReplacedDraftNotice from './ReplacedDraftNotice.vue'
import PluginsChangedNotice from './PluginsChangedNotice.vue'
import HostOfflineNotice from './HostOfflineNotice.vue'
import type { DeviceStart } from '../devices/installation'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { PlaceholderText } from '../ui/ui-text'
import { useTouchOnly } from '../ui/touch-only'

const props = withDefaults(
  defineProps<{
    placeholder: PlaceholderText
    /**
     * A turn runs, or the message that starts one is on its way: Stop shows
     * from the send to the turn's end, and a message is queued meanwhile.
     */
    running?: boolean
    compacting?: boolean
    disabled?: boolean
    attachments?: ComposerAttachment[]
    messageEdit?: MessageEditState | null
    /** Uploads a file the edit composer adds, as the main composer's files are uploaded. */
    upload: UploadFile
    /** The conversation has a host with files: the menu offers a remote file beside local ones. */
    remoteFiles?: boolean
    focused?: boolean
    /**
     * The draft takes the focus as the composer shows, so a new or opened
     * conversation is typed into at once (`product.md` § Conversations and
     * projects); not on a touch phone, where it would raise the on-screen
     * keyboard, nor while another field holds the focus.
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
    /** When no model can send, show Configure Models. Hide the action if the user cannot open that page. */
    canConfigure?: boolean
    /** Replaces the input with the archive bar. */
    archived?: boolean
    /**
     * Why nothing can be sent for now, such as `Cloud is resetting.`: the input
     * gives way to that line and returns, draft kept, when the caller clears it.
     */
    hold?: string | null
    /**
     * The version of the draft a later save replaced, which the composer
     * offers to restore: its Markdown and the names of its files.
     */
    replaced?: { markdown: string; fileNames: readonly string[] } | null
    /** The conversation's primary Host is a paired device that is offline: says how to start its runner again. */
    offlineHost?: { name: string; start: DeviceStart } | null
    /** The conversation runs with commands of plugins since turned on or off: offers a reload. */
    pluginsChanged?: boolean
    /** The reload the composer offered is under way. */
    reloading?: boolean
    /**
     * Counts the drafts shown from outside, another page's or a restored
     * one: when it changes, the editor shows the draft and its files anew,
     * even when the text is the same.
     */
    draftShown?: number
  }>(),
  {
    attachments: () => [],
    canConfigure: true,
  },
)
const draft = defineModel<string>('draft', { default: '' })
const emit = defineEmits<{
  retryModels: []
  submit: []
  stop: []
  compact: []
  /** Files to attach; their capsules land where they were dropped, pasted or picked. */
  addFiles: [files: File[]]
  /** Open the host's file browser; the caller attaches what it returns, and its capsule lands at the cursor. */
  attachRemote: []
  /** The files the message now carries, in the order of their capsules. */
  arrangeAttachments: [ids: string[]]
  /** Try a failed upload again. */
  retryAttachment: [id: string]
  /** One change of the model settings, naming only the parts it changes. */
  changeModel: [change: ModelSettingsChange]
  /** The user's context limit on a model, for all their conversations with it. */
  changeContextLimit: [change: ContextLimitChange]
  configure: []
  restore: []
  'update:messageEdit': [state: MessageEditState | null]
  submitEdit: []
  /** Bring back the replaced version, in exchange for the draft. */
  restoreReplaced: []
  dismissReplaced: []
  /** Open the conversation again with the plugins the user has on. */
  reloadPlugins: []
}>()
/**
 * The edit the composer shows: none while Regenerate sends one, which never
 * opens the editor; the draft stays, and waits for that edit to be answered.
 */
const regenerating = computed(() => props.messageEdit?.phase === 'regenerating')
const shownEdit = computed(() => regenerating.value ? null : props.messageEdit ?? null)
const edit = useMessageEditComposer({
  state: () => shownEdit.value,
  update: (state) => emit('update:messageEdit', state),
  upload: (file, options) => props.upload(file, options),
})
/** Up Arrow in the empty draft opens the editor on the last message, where the page offers editing it. */
const editLastMessage = useEditLastMessage()
const editLast = computed(() => props.disabled ? undefined : editLastMessage())
/** Read once, as the composer shows: the focus it finds then decides. */
const touchOnly = useTouchOnly()
const takesFocus = props.focusOnShow && !touchOnly.value && !isFocusedElementEditable()
/** The editor shown: the edit's, or the draft's. */
const editor = ref<InstanceType<typeof MessageEditor>>()
const focused = ref(false)
// An editor taken away while it has the focus tells no blur; the one in its place starts unfocused.
watch(() => shownEdit.value?.request.operationId, () => {
  focused.value = false
})
/**
 * The user sent the edit shown. Its editor takes no typing while it is sent,
 * and gives way to the draft's once the edit ends: the draft's takes the
 * focus then, so the next message is typed at once, as after a send, unless
 * the user has put the focus elsewhere meanwhile.
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
/** The send button, which says why it cannot send when the message is given to it anyway. */
const sendButton = ref<InstanceType<typeof IconButton>>()
/** What the composer carries, as the editor builds its document from it. */
const carried = computed(() => props.attachments.map(composerCapsule))
/**
 * The files the message has, in the order of their capsules: the document
 * says so. It starts as the editor builds it, with the files the draft's marks
 * stand for, and not those the composer still holds for an undo; from then on
 * the editor tells every change.
 */
const capsules = ref<MessageCapsule[]>(
  serializeUserMarkdown<MessageCapsule>(parseUserMarkdown(draft.value, carried.value)).attachments,
)
/** Their transfers, and those of the files an edit adds, which the capsules in the editor read. */
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
/** The files of the message, not the ones the composer still holds for an undo. */
const carrying = computed(() => {
  const ids = new Set(capsules.value.map((capsule) => capsule.id))
  return props.attachments.filter((item) => ids.has(item.id))
})
const hasDraft = computed(
  () => shownEdit.value ? editHasContent(shownEdit.value)
    : !!draft.value.trim() || !!capsules.value.length,
)
const modelState = computed(() =>
  composerModel(
    props.providers,
    props.models,
    props.modelSettings?.providerId,
    props.modelSettings?.modelId,
  ),
)
const sendDisabled = computed(
  () =>
    props.disabled ||
    regenerating.value ||
    modelState.value.kind !== 'ready' ||
    (shownEdit.value
      ? shownEdit.value.phase === 'sending' || !!edit.sendBlockReason.value
      : !attachmentsReady(carrying.value)),
)
const sendBlockReason = computed(() => {
  if (modelState.value.kind === 'unavailable') {
    return 'This model is unavailable. Choose another to send.'
  }
  if (props.disabled) {
    return undefined
  }
  return shownEdit.value
    ? edit.sendBlockReason.value
    : attachmentSendBlockReason(carrying.value.filter(isComposerFile).map((item) => item.phase))
})
// The composer shows no failure text of its own: a file an edit could not
// take or upload is a toast, a failed upload is its capsule's Retry, and a
// refused edit is the product's toast.
watch(edit.attachmentError, (message) => {
  if (message) {
    showToast({ title: 'Couldn’t Attach', message, tone: 'danger' })
  }
})
/** The replaced version as the offer shows it; none while there is none, or while a message is edited. */
const replacedPreview = computed(() =>
  props.replaced && !shownEdit.value
    ? draftPreview(props.replaced.markdown, props.replaced.fileNames)
    : null,
)
/** Why Compact cannot run now apart from how full the context is: the session takes it only while idle. */
const compactUnavailable = computed(() => {
  if (shownEdit.value) {
    return 'Compaction is available after the edit.'
  }
  if (props.running) {
    return 'Compaction is available once the turn ends.'
  }
  return null
})
const submitLabel = computed(() => shownEdit.value
  ? shownEdit.value.phase === 'uncertain' ? 'Retry' : 'Save and resend'
  : props.running ? 'Queue' : 'Send',
)

function submit() {
  if (sendDisabled.value) {
    // Enter on a message that cannot go: the send button says why, unpointed.
    sendButton.value?.showReason()
    return
  }
  if (!hasDraft.value) {
    return
  }
  if (shownEdit.value) {
    editSent = true
    emit('submitEdit')
    return
  }
  emit('submit')
  // The next message is typed at once, wherever the focus was when it was sent.
  editor.value?.focus()
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
  // The menu that opened the chooser took the focus with it: the message gets it back, to go on typing.
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
  // An edit uploads its files itself; their capsules go in where they were told to land.
  insertCapsules(edit.addFiles(files))
}

/** Puts files the host took into the message, where the composer said they would land. */
function insertCapsules(added: readonly MessageCapsule[]): void {
  editor.value?.insertCapsules(added)
}

defineExpose({
  /** Files the host has taken: their capsules go into the message where they were told to land. */
  insertCapsules,
  /** Files are on their way: the next capsules land at the cursor. */
  placeNextFiles(event?: DragEvent): void {
    editor.value?.placeNextFiles(event)
  },
  /**
   * The user asked for this draft again, as New does on the draft it
   * shows: the message takes the focus, except on a touch phone.
   */
  focusDraft(): void {
    if (!touchOnly.value) {
      editor.value?.focus()
    }
  },
})

function changeDraft(markdown: string, attachments: MessageCapsule[]): void {
  draft.value = markdown
  const had = capsules.value.map((capsule) => capsule.id).join('\n')
  capsules.value = attachments
  const ids = attachments.map((capsule) => capsule.id)
  // The host hears which files the message has only when that changes; its text it hears every time.
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
      label="This conversation is archived."
      action="Restore Conversation"
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
      label="No models available."
      :action="
        canConfigure !== false ? 'Configure Models' : undefined
      "
      @action="emit('configure')"
    />
    <div v-else key="composer" class="w-full">
      <input
        ref="fileInput"
        type="file"
        class="hidden"
        multiple
        @change="fileChange"
      />
      <HostOfflineNotice
        v-if="offlineHost"
        class="mb-2"
        :name="offlineHost.name"
        :start="offlineHost.start"
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
      <ComposerShell
        :focused="focused || props.focused"
        :expanded="multiline"
        :dropping="dropping"
        @drop-files="dropFiles"
      >
        <template #editor="{ line }">
          <MessageEditor
            v-if="shownEdit"
            :key="shownEdit.request.operationId"
            ref="editor"
            v-model:multiline="multiline"
            composer
            cancelable
            autofocus
            :line-width="line"
            :disabled="!edit.editable.value"
            :markdown="edit.markdown.value"
            :attachments="edit.capsules.value"
            :placeholder="placeholder"
            label="Message"
            @change="edit.change"
            @submit="submit"
            @cancel="edit.cancel"
            @files="addFiles"
            @focus="focused = true"
            @blur="focused = false"
          />
          <MessageEditor
            v-else
            ref="editor"
            v-model:multiline="multiline"
            composer
            :line-width="line"
            :markdown="draft"
            :attachments="carried"
            :shown-from-outside="draftShown"
            :autofocus="takesFocus"
            :placeholder="placeholder"
            :edit-last="editLast"
            label="Message"
            @change="changeDraft"
            @submit="submit"
            @files="addFiles"
            @focus="focused = true"
            @blur="focused = false"
          />
        </template>
        <template #attach>
          <Dropdown
            v-if="!shownEdit || edit.editable.value"
            :overlay-store="appOverlayStore"
            :placement="attachOpen ? 'bottom-start' : 'top-start'"
            v-bind="attachOpen ? { open: true } : {}"
          >
            <template #trigger="{ isOpen }">
              <!-- A press on + takes no focus, as on the send button: the message keeps it until the
                   menu takes it, and gets it back when the menu closes. -->
              <Tooltip content="Attach">
                <IconButton
                  :icon="Plus"
                  variant="ghost"
                  circle
                  :pressed="isOpen"
                  aria-label="Attach"
                  @mousedown.prevent
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  :icon="FileIcon"
                  :label="remoteFiles ? 'Attach Local Files…' : 'Attach Files…'"
                  @select="pickFiles(close)"
                />
                <MenuItem
                  v-if="remoteFiles && !shownEdit"
                  :icon="HardDrive"
                  label="Attach Remote File…"
                  @select="attachRemote"
                />
              </Menu>
            </template>
          </Dropdown>
        </template>
        <template #model>
          <div :inert="!!shownEdit && !edit.editable.value">
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
        </template>
        <template #actions>
          <ContextUsageIndicator
            :usage="usage"
            :is-compacting="compacting"
            :unavailable-reason="compactUnavailable"
            @compact="emit('compact')"
          />
          <Tooltip v-if="shownEdit" content="Cancel edit">
            <IconButton
              :icon="X"
              variant="ghost"
              circle
              aria-label="Cancel edit"
              :disabled="!edit.editable.value"
              :tabindex="edit.editable.value ? 0 : -1"
              @click="edit.cancel"
              @keydown.enter.space.prevent="edit.cancel"
            />
          </Tooltip>
          <!-- Stop stays for the whole turn, beside Queue while the message holds text. -->
          <Tooltip v-if="(running || compacting) && !shownEdit" content="Stop">
            <IconButton
              :icon="Square"
              variant="ghost"
              circle
              aria-label="Stop"
              @mousedown.prevent
              @click="emit('stop')"
            />
          </Tooltip>
          <!-- A press on the send button takes no focus: the message keeps it. -->
          <Tooltip
            v-if="hasDraft"
            :content="submitLabel"
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
              @click="submit"
            />
          </Tooltip>
          <IconButton
            v-else-if="!running && !compacting"
            :icon="ArrowUp"
            variant="ghost"
            circle
            disabled
            aria-label="Send"
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
