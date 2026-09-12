<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { ThinkingConfig, TokenUsage } from '@demicodes/core'
import { ArrowUp, File as FileIcon, HardDrive, Plus, RotateCcw, Square, X } from '@lucide/vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '../overlay/appOverlay'
import { t } from '../infra/i18n'
import AttachmentTile from './AttachmentTile.vue'
import {
  attachmentsReady,
  attachmentCaption,
  attachmentSendBlockReason,
  composerFileNames,
  dataTransferFiles,
  pastedTextFile,
  pastedTextIsLong,
  type ComposerAttachment,
  decodeRemoteReference,
} from './message-input/attachments'
import { useMessageEditComposer } from './message-input/useMessageEditComposer'
import { shouldSubmitFromEditorKeydown } from './message-input/composer-keyboard'
import { editHasContent, type MessageEditState } from './message-editing'
import ContentMedia from './ContentMedia.vue'
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
    conversationId?: string
    running?: boolean
    compacting?: boolean
    disabled?: boolean
    hasContent?: boolean
    multiline?: boolean
    canCompact?: boolean
    attachments?: ComposerAttachment[]
    messageEdit?: MessageEditState | null
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
  }>(),
  {
    attachments: () => [],
    canCompact: true,
    canConfigure: true,
  },
)
const draft = defineModel<string>('draft', { default: '' })
const emit = defineEmits<{
  retryModels: []
  submit: []
  stop: []
  compact: []
  addFiles: [files: File[]]
  /** Open the host's file browser; the caller attaches what it returns. */
  attachRemote: []
  removeAttachment: [id: string]
  selectModel: [providerId: string, modelId: string]
  changeThinking: [config: ThinkingConfig]
  changeServiceTier: [id: string | null]
  configure: []
  restore: []
  'update:messageEdit': [state: MessageEditState | null]
  submitEdit: []
}>()
const root = ref<HTMLElement>()
const edit = useMessageEditComposer({
  state: () => props.messageEdit,
  update: (state) => emit('update:messageEdit', state),
  root,
})
const focused = ref(false)
const fileInput = ref<HTMLInputElement>()
const hasDraft = computed(
  () => props.messageEdit ? editHasContent(props.messageEdit)
    : props.hasContent || !!draft.value.trim() || !!props.attachments.length,
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
      ? props.messageEdit.phase === 'sending' || !!edit.reading.value
      : !attachmentsReady(props.attachments)),
)
const sendBlockReason = computed(() => {
  if (modelState.value.kind === 'unavailable') {
    return t('agent.input.switchModel')
  }
  if (props.disabled) {
    return undefined
  }
  return props.messageEdit ? undefined : attachmentSendBlockReason(props.attachments)
})
// The composer shows no failure text of its own: a file that could not be
// read for an edit is a toast, a failed upload is the tile's Retry, and a
// refused edit is the product's toast.
watch(edit.attachmentError, (message) => {
  if (message) {
    showToast({ title: t('agent.input.attachmentFailed'), message, tone: 'danger' })
  }
})
const expanded = computed(
  () =>
    props.messageEdit
      ? edit.textParts.value.length > 1 || edit.textParts.value.some(({ part }) => part.type === 'text' && part.text.includes('\n')) || !!edit.attachments.value.length
      : props.multiline || draft.value.includes('\n') || !!props.attachments.length,
)
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
  if (!sendDisabled.value && hasDraft.value) {
    if (props.messageEdit) {
      emit('submitEdit')
    } else {
      emit('submit')
    }
  }
}

function pickFiles(close: () => void) {
  close()
  fileInput.value?.click()
}

function fileChange(event: Event) {
  const input = event.target as HTMLInputElement
  addFiles([...(input.files ?? [])])
  input.value = ''
}

// Files on the clipboard attach; a long text pastes as a file. A short text
// stays the editor's own paste.
function paste(event: ClipboardEvent) {
  const transfer = event.clipboardData
  if (!transfer) {
    return
  }
  const files = dataTransferFiles(transfer)
  if (files.length > 0) {
    event.preventDefault()
    addFiles(files)
    return
  }
  const text = transfer.getData('text/plain')
  if (!pastedTextIsLong(text)) {
    return
  }
  event.preventDefault()
  const names = props.messageEdit
    ? edit.attachments.value.flatMap(({ part }) => (part.type === 'document' && part.source.fileName ? [part.source.fileName] : []))
    : composerFileNames(props.attachments)
  addFiles([pastedTextFile(text, names)])
}

function keydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && props.messageEdit && !event.isComposing) {
    event.preventDefault()
    edit.cancel()
    return
  }
  if (!shouldSubmitFromEditorKeydown(event)) {
    return
  }
  event.preventDefault()
  submit()
}

function addFiles(files: File[]): void {
  if (props.messageEdit) {
    void edit.addFiles(files)
  } else {
    emit('addFiles', files)
  }
}
</script>

<template>
  <Transition name="composer-archive" mode="out-in">
    <SessionNoticeBar
      v-if="archived"
      key="archived"
      :label="t('agent.session.archived')"
      :action="t('agent.session.restore')"
      @action="emit('restore')"
    />
    <SessionNoticeBar
      v-else-if="
        modelState.kind === 'none' && (!modelLoad || modelLoad === 'ready')
      "
      key="none"
      :label="t('agent.input.noModels')"
      :action="
        canConfigure !== false ? t('agent.input.configureModels') : undefined
      "
      @action="emit('configure')"
    />
    <div v-else key="composer" ref="root" class="w-full">
      <input
        ref="fileInput"
        type="file"
        class="hidden"
        multiple
        @change="fileChange"
      />
      <ComposerShell
        :focused="focused || props.focused"
        :expanded="expanded"
        :dropping="dropping"
        @drop-files="addFiles"
      >
        <template v-if="messageEdit ? edit.attachments.value.length : attachments.length" #chips>
          <template v-if="messageEdit">
            <template v-for="{ part, index } in edit.attachments.value" :key="index">
              <AttachmentTile
                v-if="part.type === 'reference'"
                :name="decodeRemoteReference(part.reference).name"
                :removable="edit.editable.value"
                @remove="edit.removeAttachment(index)"
              />
              <AttachmentTile
                v-else-if="part.type === 'attachment'"
                :name="part.name"
                :snippet="part.snippet"
                :removable="edit.editable.value"
                @remove="edit.removeAttachment(index)"
              />
              <ContentMedia
                v-else-if="part.type !== 'text'"
                :kind="part.type"
                :source="part.source"
                :name="'fileName' in part.source ? part.source.fileName ?? part.type : part.type"
                as-attachment
                :removable="edit.editable.value"
                @remove="edit.removeAttachment(index)"
              />
            </template>
          </template>
          <template v-else>
            <Tooltip
              v-for="item in attachments"
              :key="item.id"
              :content="attachmentCaption(item)"
            >
              <AttachmentTile
                :name="item.name"
                :src="item.kind === 'file' ? item.src : undefined"
                :snippet="item.kind === 'file' ? item.snippet : undefined"
                :phase="item.kind === 'file' ? item.phase : undefined"
                :progress="item.kind === 'file' ? item.progress : undefined"
                removable
                @remove="emit('removeAttachment', item.id)"
              />
            </Tooltip>
          </template>
        </template>
        <template #editor>
          <div v-if="messageEdit" class="message-edit-texts flex w-full flex-col gap-2">
            <template v-for="{ part, index } in edit.textParts.value" :key="index">
              <textarea
                v-if="part.type === 'text'"
                :value="part.text"
                :disabled="!edit.editable.value"
                :aria-label="edit.textParts.value.length === 1 ? 'Message' : `Message text ${index + 1}`"
                :placeholder="placeholder"
                rows="1"
                class="w-full resize-none bg-transparent text-conversation text-fg outline-none placeholder:text-fg-subtle"
                @input="edit.changeText(index, ($event.target as HTMLTextAreaElement).value)"
                @focus="focused = true"
                @blur="focused = false"
                @keydown="keydown"
                @paste="paste"
              />
            </template>
          </div>
          <slot v-else name="editor">
            <textarea
              v-model="draft"
              rows="1"
              aria-label="Message"
              :placeholder="placeholder"
              class="w-full resize-none bg-transparent text-conversation text-fg outline-none placeholder:text-fg-subtle"
              @focus="focused = true"
              @blur="focused = false"
              @keydown="keydown"
              @paste="paste"
            />
          </slot>
        </template>
        <template #attach>
          <Dropdown
            v-if="!messageEdit || edit.editable.value"
            :overlay-store="appOverlayStore"
            :placement="attachOpen ? 'bottom-start' : 'top-start'"
            v-bind="attachOpen ? { open: true } : {}"
          >
            <template #trigger="{ isOpen }">
              <Tooltip :content="t('agent.input.attach')">
                <IconButton
                  :icon="Plus"
                  variant="ghost"
                  circle
                  :pressed="isOpen"
                  :aria-label="t('agent.input.attach')"
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  :icon="FileIcon"
                  :label="
                    t(
                      remoteFiles
                        ? 'agent.input.attachLocalFiles'
                        : 'agent.input.attachFiles',
                    )
                  "
                  @select="pickFiles(close)"
                />
                <MenuItem
                  v-if="remoteFiles && !messageEdit"
                  :icon="HardDrive"
                  :label="t('agent.input.attachRemoteFile')"
                  @select="emit('attachRemote')"
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
            :conversation-id="conversationId"
            :usage="usage"
            :context-window="selected?.contextWindow"
            :input-limit="selected?.inputLimit"
            :is-compacting="compacting"
            :is-clickable="!messageEdit && !running && canCompact !== false"
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
              :icon="messageEdit?.phase === 'uncertain' ? RotateCcw : ArrowUp"
              variant="accent"
              circle
              :disabled="sendDisabled"
              :loading="messageEdit?.phase === 'sending' || !!edit.reading.value"
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
:deep(.composer-editor:has(.message-edit-texts)) {
  overflow-y: auto;
}

.message-edit-texts textarea {
  min-height: var(--composer-line);
  max-height: none;
}

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
