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
  dataTransferFiles,
  filePreviewUrl,
  fileToUserContent,
  partitionAcceptedFiles,
} from './message-input/attachments'

interface ComposerAttachment {
  name: string
  block: UserContentBlock
  previewUrl?: string
}

const props = defineProps<{
  conversationId: string
}>()

const emit = defineEmits<{
  'empty-submit': []
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

const attachments = ref<ComposerAttachment[]>([])
const isMultiline = ref(false)

function buildSubmitPayload(): UserContentBlock[] | null {
  const currentEditor = editor.value
  const attached = attachments.value.map((item) => item.block)
  if ((!currentEditor || currentEditor.isEmpty) && attached.length === 0) return null
  const content = docToContent(currentEditor?.getJSON() as InputModel | undefined, attached)
  return content.length > 0 ? content : null
}

function revokePreview(item: ComposerAttachment): void {
  if (item.previewUrl) URL.revokeObjectURL(item.previewUrl)
}

function clearInput(): void {
  editor.value?.commands.clearContent()
  for (const item of attachments.value) revokePreview(item)
  attachments.value = []
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
  const { accepted, rejected } = partitionAcceptedFiles(files, acceptedExtensions.value)
  if (rejected.length > 0) {
    showToast({
      title: t('agent.input.unsupportedFiles'),
      message: rejected.map((file) => file.name).join(', '),
      tone: 'danger',
    })
  }
  if (accepted.length === 0) return
  const next = await Promise.all(
    accepted.map(async (file) => {
      const block = await fileToUserContent(file)
      return {
        name: file.name,
        block,
        previewUrl: block.type === 'image' ? filePreviewUrl(file) : undefined,
      }
    }),
  )
  attachments.value = [...attachments.value, ...next]
}

// Claims the paste only when something was attached; otherwise the editor keeps its default
// (the clipboard's text alternative) instead of swallowing the paste.
function handlePasteAttachments(clipboardData: DataTransfer, _text: string): boolean {
  const files = dataTransferFiles(clipboardData)
  if (files.length === 0) return false
  void addFiles(files)
  return partitionAcceptedFiles(files, acceptedExtensions.value).accepted.length > 0
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

function removeAttachment(index: number): void {
  const item = attachments.value[index]
  if (item) revokePreview(item)
  attachments.value = attachments.value.filter((_, itemIndex) => itemIndex !== index)
}

function attachmentName(item: ComposerAttachment): string {
  if (item.block.type === 'document') return item.block.source.fileName
  return item.name
}

onBeforeUnmount(() => {
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
const displayAttachments = computed(() =>
  attachments.value.map((item) => ({
    name: attachmentName(item),
    src: item.previewUrl,
  })),
)
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
