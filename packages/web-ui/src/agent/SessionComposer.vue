<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { ThinkingConfig, TokenUsage } from '@demicodes/protocol'
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
import { editHasContent, type MessageEditState } from './message-editing'
import { composerCapsule, composerTransfer, provideTransfers, type MessageCapsule } from './message-editor/capsules'
import MessageEditor from './message-editor/MessageEditor.vue'
import { showToast } from '../infra/toast'
import ComposerShell from './ComposerShell.vue'
import ContextUsageIndicator from './ContextUsageIndicator.vue'
import ModelSelector from './ModelSelector.vue'
import { composerModel } from './model-selection'
import SessionNoticeBar from './SessionNoticeBar.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Tooltip from '../ui/Tooltip.vue'

const props = withDefaults(
  defineProps<{
    placeholder: string
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
    attachOpen?: boolean
    dropping?: boolean
    modelLoad?: 'loading' | 'ready' | 'failed'
    providers: ProviderInfo[]
    models: Record<string, ModelInfo[]>
    selectedProviderId?: string | null
    selectedModelId?: string | null
    thinkingConfig?: ThinkingConfig
    serviceTierId?: string | null
    usage?: TokenUsage | null
    /** When no model can send, show Configure models. Hide the action if the user cannot open that page. */
    canConfigure?: boolean
    /** Replaces the input with the archive bar. */
    archived?: boolean
    /**
     * Why nothing can be sent for now, such as `Cloud is resetting.`: the input
     * gives way to that line and returns, draft kept, when the caller clears it.
     */
    hold?: string | null
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
  selectModel: [providerId: string, modelId: string]
  changeThinking: [config: ThinkingConfig]
  changeServiceTier: [id: string | null]
  configure: []
  restore: []
  'update:messageEdit': [state: MessageEditState | null]
  submitEdit: []
}>()
const edit = useMessageEditComposer({
  state: () => props.messageEdit,
  update: (state) => emit('update:messageEdit', state),
  upload: (file, options) => props.upload(file, options),
})
/** The editor shown: the edit's, or the draft's. */
const editor = ref<InstanceType<typeof MessageEditor>>()
const focused = ref(false)
// An editor taken away while it has the focus tells no blur; the one in its place starts unfocused.
watch(() => props.messageEdit?.request.operationId, () => {
  focused.value = false
})
/** The message holds more than one line, so the composer grows to hold it. */
const multiline = ref(false)
const fileInput = ref<HTMLInputElement>()
/** The send button, which says why it cannot send when the message is given to it anyway. */
const sendButton = ref<InstanceType<typeof IconButton>>()
/** What the composer carries, as the editor builds its document from it. */
const carried = computed(() => props.attachments.map(composerCapsule))
/** The files the message has, in the order of their capsules: the document says so. */
const capsules = ref<MessageCapsule[]>(carried.value)
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
  () => props.messageEdit ? editHasContent(props.messageEdit)
    : !!draft.value.trim() || !!capsules.value.length,
)
const modelState = computed(() =>
  composerModel(
    props.providers,
    props.models,
    props.selectedProviderId,
    props.selectedModelId,
  ),
)
const sendDisabled = computed(
  () =>
    props.disabled ||
    modelState.value.kind !== 'ready' ||
    (props.messageEdit
      ? props.messageEdit.phase === 'sending' || !!edit.sendBlockReason.value
      : !attachmentsReady(carrying.value)),
)
const sendBlockReason = computed(() => {
  if (modelState.value.kind === 'unavailable') {
    return 'This model is unavailable. Choose another to send.'
  }
  if (props.disabled) {
    return undefined
  }
  return props.messageEdit
    ? edit.sendBlockReason.value
    : attachmentSendBlockReason(carrying.value.filter(isComposerFile).map((item) => item.phase))
})
// The composer shows no failure text of its own: a file an edit could not
// take or upload is a toast, a failed upload is its capsule's Retry, and a
// refused edit is the product's toast.
watch(edit.attachmentError, (message) => {
  if (message) {
    showToast({ title: "Couldn't attach", message, tone: 'danger' })
  }
})
const selected = computed(() =>
  props.models[props.selectedProviderId ?? '']?.find(
    (model) => model.id === props.selectedModelId,
  ),
)
const submitLabel = computed(() => props.messageEdit
  ? props.messageEdit.phase === 'uncertain' ? 'Retry' : 'Save and resend'
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
  if (props.messageEdit) {
    emit('submitEdit')
  } else {
    emit('submit')
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
  if (!props.messageEdit) {
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
      action="Restore conversation"
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
        canConfigure !== false ? 'Configure models' : undefined
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
      <ComposerShell
        :focused="focused || props.focused"
        :expanded="multiline"
        :dropping="dropping"
        @drop-files="dropFiles"
      >
        <template #editor="{ line }">
          <MessageEditor
            v-if="messageEdit"
            :key="messageEdit.request.operationId"
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
            :placeholder="placeholder"
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
            v-if="!messageEdit || edit.editable.value"
            :overlay-store="appOverlayStore"
            :placement="attachOpen ? 'bottom-start' : 'top-start'"
            v-bind="attachOpen ? { open: true } : {}"
          >
            <template #trigger="{ isOpen }">
              <Tooltip content="Attach">
                <IconButton
                  :icon="Plus"
                  variant="ghost"
                  circle
                  :pressed="isOpen"
                  aria-label="Attach"
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  :icon="FileIcon"
                  :label="remoteFiles ? 'Attach local files' : 'Attach files'"
                  @select="pickFiles(close)"
                />
                <MenuItem
                  v-if="remoteFiles && !messageEdit"
                  :icon="HardDrive"
                  label="Attach remote file…"
                  @select="attachRemote"
                />
              </Menu>
            </template>
          </Dropdown>
        </template>
        <template #model>
          <div :inert="!!messageEdit && !edit.editable.value">
            <ModelSelector
              :load="modelLoad"
              @retry="emit('retryModels')"
              :providers="providers"
              :models="models"
              :selected-provider-id="selectedProviderId"
              :selected-model-id="selectedModelId"
              :thinking-config="thinkingConfig"
              :service-tier-id="serviceTierId"
              @select-model="
                (provider, model) => emit('selectModel', provider, model)
              "
              @change-thinking="emit('changeThinking', $event)"
              @change-service-tier="emit('changeServiceTier', $event)"
            />
          </div>
        </template>
        <template #actions>
          <ContextUsageIndicator
            :usage="usage"
            :context-window="selected?.contextWindow"
            :input-limit="selected?.inputLimit"
            :is-compacting="compacting"
            :is-clickable="!messageEdit && !running"
            @compact="emit('compact')"
          />
          <Tooltip v-if="messageEdit" content="Cancel edit">
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
          <Tooltip
            v-if="hasDraft"
            :content="submitLabel"
            :disabled="sendDisabled"
          >
            <IconButton
              ref="sendButton"
              :icon="messageEdit?.phase === 'uncertain' ? RotateCcw : ArrowUp"
              variant="accent"
              circle
              :disabled="sendDisabled"
              :loading="messageEdit?.phase === 'sending'"
              :disabled-reason="sendBlockReason"
              :aria-label="submitLabel"
              @click="submit"
            />
          </Tooltip>
          <Tooltip v-else-if="running || compacting" content="Stop">
            <IconButton
              :icon="Square"
              variant="ghost"
              circle
              aria-label="Stop"
              @click="emit('stop')"
            />
          </Tooltip>
          <IconButton
            v-else
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
