<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import type { UserContentBlock } from '@demicodes/core'
import { ArrowUp, File as FileIcon, Plus, Square } from '@lucide/vue'
import { EditorContent } from '@tiptap/vue-3'
import { useAgentWorkspace } from './workspace'
import AttachmentTile from './AttachmentTile.vue'
import ComposerShell from './ComposerShell.vue'
import ModelSelector from './ModelSelector.vue'
import ContextUsageIndicator from './ContextUsageIndicator.vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { t } from '../infra/i18n'
import { showToast } from '../infra/toast'
import { useAgentInputActions } from './message-input/useAgentInputActions'
import { useAgentInputEditor } from './message-input/useAgentInputEditor'
import { useAgentInputSessionState } from './message-input/useAgentInputSessionState'
import { docToContent, type InputModel } from './message-input/input-model'
import { composerHasLineBreak } from './message-input/composer-multiline'
import { acceptAttribute, dataTransferFiles, filePreviewUrl, fileToUserContent, partitionAcceptedFiles } from './message-input/attachments'

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
  contextWindow,
  inputLimit,
  acceptedExtensions,
  isRunning,
  isCompacting,
  canCompact,
  usage,
} = useAgentInputSessionState(workspace, props.conversationId)

const attachments = ref<ComposerAttachment[]>([])
const isMultiline = ref(false)
const fileInputRef = ref<HTMLInputElement>()
const expanded = computed(() => isMultiline.value || attachments.value.length > 0)

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

const { handleSubmit, handleSelectModel, handleChangeThinking, handleChangeServiceTier, handleAbort, handleCompact } = useAgentInputActions({
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
    showToast({ title: t('agent.input.unsupportedFiles'), message: rejected.map((file) => file.name).join(', '), tone: 'danger' })
  }
  if (accepted.length === 0) return
  const next = await Promise.all(accepted.map(async (file) => {
    const block = await fileToUserContent(file)
    return {
      name: file.name,
      block,
      previewUrl: block.type === 'image' ? filePreviewUrl(file) : undefined,
    }
  }))
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

watch(editor, (current) => {
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
}, { immediate: true })

function pickFiles(): void {
  fileInputRef.value?.click()
}

function onFileChange(event: Event): void {
  const input = event.target as HTMLInputElement
  const files = input.files ? [...input.files] : []
  input.value = ''
  void addFiles(files)
}

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
</script>

<template>
  <div>
    <input
      ref="fileInputRef"
      type="file"
      class="hidden"
      multiple
      :accept="acceptAttribute(acceptedExtensions)"
      @change="onFileChange"
    />
    <ComposerShell :focused="isFocused" :expanded="expanded" @drop-files="addFiles">
      <template v-if="attachments.length > 0" #chips>
        <AttachmentTile
          v-for="(item, index) in attachments"
          :key="`${item.name}-${index}`"
          :name="attachmentName(item)"
          :src="item.previewUrl"
          removable
          @remove="removeAttachment(index)"
        />
      </template>
      <template #editor>
        <EditorContent v-if="editor" :editor="editor" />
      </template>
      <template #attach>
        <Dropdown :overlay-store="appOverlayStore" placement="top-start">
          <template #trigger="{ isOpen }">
            <Tooltip :content="t('agent.input.attach')">
              <IconButton :icon="Plus" variant="ghost" circle :pressed="isOpen" />
            </Tooltip>
          </template>
          <template #content="{ close }">
            <Menu>
              <MenuItem :icon="FileIcon" :label="t('agent.input.attachFiles')" @select="close(); pickFiles()" />
            </Menu>
          </template>
        </Dropdown>
      </template>
      <template #model>
        <ModelSelector
          :providers="workspace.providers.value"
          :models="workspace.models"
          :selected-provider-id="selectedProviderId"
          :selected-model-id="selectedModelId"
          :service-tier-id="serviceTierId"
          v-bind="thinkingConfig ? { thinkingConfig } : {}"
          @select-model="handleSelectModel"
          @change-thinking="handleChangeThinking"
          @change-service-tier="handleChangeServiceTier"
        />
      </template>
      <template #actions>
        <ContextUsageIndicator
          :conversation-id="props.conversationId"
          :usage="usage"
          :context-window="contextWindow"
          :input-limit="inputLimit"
          :is-compacting="isCompacting"
          :is-clickable="!isRunning && canCompact"
          @compact="handleCompact"
        />
        <Tooltip v-if="hasContent || attachments.length > 0" :content="isRunning ? 'Queue next turn' : 'Send message'">
          <IconButton :icon="ArrowUp" variant="accent" circle @click="handleSubmit" />
        </Tooltip>
        <Tooltip v-else-if="isRunning || isCompacting" content="Stop">
          <IconButton :icon="Square" variant="ghost" circle @click="handleAbort" />
        </Tooltip>
        <IconButton v-else :icon="ArrowUp" variant="ghost" circle disabled />
      </template>
    </ComposerShell>
  </div>
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
