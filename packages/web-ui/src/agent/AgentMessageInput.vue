<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import type { UserContentBlock } from '@demicodes/core'
import { EditorContent } from '@tiptap/vue-3'
import { useAgentWorkspace } from './workspace'
import SessionComposer from './SessionComposer.vue'
import { t } from '../infra/i18n'
import { showToast } from '../infra/toast'
import { useAgentInputActions } from './message-input/useAgentInputActions'
import { useAgentInputEditor } from './message-input/useAgentInputEditor'
import { useAgentInputSessionState } from './message-input/useAgentInputSessionState'
import { docToContent, type InputModel } from './message-input/input-model'
import { composerHasLineBreak } from './message-input/composer-multiline'
import {
  acceptAttribute,
  applyAttachmentUpdate,
  AttachmentUploadQueue,
  attachmentsReady,
  composerAttachmentFromFile,
  dataTransferFiles,
  fileToUserContent,
  partitionAcceptedFiles,
  type AttachmentUploadUpdate,
  type ComposerFileAttachment,
} from './message-input/attachments'

interface HeldAttachment {
  item: ComposerFileAttachment
  file: File
  block?: UserContentBlock
}

const props = defineProps<{
  conversationId: string
}>()

const emit = defineEmits<{
  'empty-submit': []
  configure: []
}>()

const workspace = useAgentWorkspace()

const {
  selectedProviderId,
  selectedModelId,
  serviceTierId,
  thinkingConfig,
  acceptedExtensions,
  isRunning,
  isCompacting,
  canCompact,
  usage,
} = useAgentInputSessionState(workspace, props.conversationId)

const attachments = ref<HeldAttachment[]>([])
const uploads = new AttachmentUploadQueue()
const isMultiline = ref(false)

function buildSubmitPayload(): UserContentBlock[] | null {
  const currentEditor = editor.value
  const pending = attachments.value.map((item) => item.item)
  if (!attachmentsReady(pending))
    return null
  const attached = attachments.value
    .filter((item) => item.block)
    .map((item) => item.block!)
  if ((!currentEditor || currentEditor.isEmpty) && attached.length === 0)
    return null
  const content = docToContent(currentEditor?.getJSON() as InputModel | undefined, attached)
  return content.length > 0 ? content : null
}

function revokePreview(item: HeldAttachment): void {
  if (item.item.src)
    URL.revokeObjectURL(item.item.src)
}

function clearInput(): void {
  editor.value?.commands.clearContent()
  uploads.cancelAll()
  for (const item of attachments.value) revokePreview(item)
  attachments.value = []
}

function applyUpdate(id: string, update: AttachmentUploadUpdate): void {
  const held = attachments.value.find((item) => item.item.id === id)
  if (held)
    applyAttachmentUpdate(held.item, update)
}

function dropFailed(id: string): void {
  const held = attachments.value.find((item) => item.item.id === id)
  if (held) {
    showToast({
      title: t('agent.input.attachmentFailed'),
      message: held.item.name,
      tone: 'danger',
    })
  }
  removeAttachment(id)
}

const {
  handleSubmit,
  handleSelectModel,
  handleChangeThinking,
  handleChangeServiceTier,
  handleAbort,
  handleCompact,
} = useAgentInputActions({
  workspace,
  conversationId: props.conversationId,
  buildSubmitPayload,
  clearInput,
  emitEmptySubmit() {
    emit('empty-submit')
  },
})

async function addFiles(files: File[]): Promise<void> {
  for (const file of files) {
    const item = composerAttachmentFromFile(file)
    const held: HeldAttachment = { item, file }
    attachments.value = [...attachments.value, held]
    readAttachment(held)
  }
}

function readAttachment(held: HeldAttachment): void {
  uploads.start(
    held.item.id,
    async (signal, report) => {
      const block = await fileToUserContent(held.file, { signal, onProgress: report })
      if (signal.aborted)
        return
      held.block = block
    },
    (update) => applyUpdate(held.item.id, update),
    dropFailed,
  )
}

// Claims the paste only when something was attached; otherwise the editor keeps its default
// (the clipboard's text alternative) instead of swallowing the paste.
function handlePasteAttachments(clipboardData: DataTransfer, _text: string): boolean {
  const files = dataTransferFiles(clipboardData)
  if (files.length === 0)
    return false
  void addFiles(files)
  return true
}

const { editor, isFocused, hasContent } = useAgentInputEditor({
  handleSubmit,
  handleCancel() {},
  handlePasteAttachments,
})

watch(
  editor,
  (current) => {
    if (!current) {
      isMultiline.value = false
      return
    }
    const sync = () => {
      isMultiline.value = composerHasLineBreak(
        current.state.doc.childCount,
        current.getText({ blockSeparator: '\n' }),
      )
    }
    sync()
    current.on('update', sync)
  },
  { immediate: true },
)

function removeAttachment(id: string): void {
  const item = attachments.value.find((held) => held.item.id === id)
  if (!item)
    return
  uploads.cancel(id)
  revokePreview(item)
  attachments.value = attachments.value.filter((held) => held.item.id !== id)
}

onBeforeUnmount(() => {
  uploads.cancelAll()
  for (const item of attachments.value) revokePreview(item)
})

defineExpose({
  focus() {
    editor.value?.commands.focus('end', { scrollIntoView: false })
  },
  prefill(content: InputModel) {
    editor.value?.commands.setContent(content)
    nextTick(() => editor.value?.commands.focus('end', { scrollIntoView: false }))
  },
})
const displayAttachments = computed(() => attachments.value.map((held) => held.item))
</script>

<template>
  <SessionComposer
    placeholder="Ask Demi…"
    :conversation-id="conversationId"
    :focused="isFocused"
    :multiline="isMultiline"
    :has-content="hasContent"
    :attachments="displayAttachments"
    :accept="acceptAttribute(acceptedExtensions)"
    :providers="workspace.providers.value"
    :models="workspace.models"
    :selected-provider-id="selectedProviderId"
    :selected-model-id="selectedModelId"
    :thinking-config="thinkingConfig"
    :service-tier-id="serviceTierId"
    :usage="usage"
    :running="isRunning"
    :compacting="isCompacting"
    :can-compact="canCompact"
    @submit="handleSubmit"
    @configure="emit('configure')"
    @add-files="addFiles"
    @remove-attachment="removeAttachment"
    @select-model="handleSelectModel"
    @change-thinking="handleChangeThinking"
    @change-service-tier="handleChangeServiceTier"
    @stop="handleAbort"
    @compact="handleCompact"
  >
    <template #editor>
      <EditorContent v-if="editor" :editor="editor" />
    </template>
  </SessionComposer>
</template>

<style>
.tiptap p.is-editor-empty:first-child::before {
  content: attr(data-placeholder);
  float: left;
  color: var(--color-fg-subtle);
  pointer-events: none;
  height: 0;
}
</style>
