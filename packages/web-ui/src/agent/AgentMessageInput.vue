<script setup lang="ts">
import { nextTick } from 'vue'
import type { UserContentBlock } from '@demicodes/core'
import { AddLine } from '@mingcute/vue/add'
import { SendLine } from '@mingcute/vue/send'
import { StopLine } from '@mingcute/vue/stop'
import { EditorContent } from '@tiptap/vue-3'
import { useAgentWorkspace } from './workspace'
import ModelSelector from './ModelSelector.vue'
import ContextUsageIndicator from './ContextUsageIndicator.vue'
import Tooltip from '../ui/Tooltip.vue'
import { useAgentInputActions } from './message-input/useAgentInputActions'
import { useAgentInputEditor } from './message-input/useAgentInputEditor'
import { useAgentInputSessionState } from './message-input/useAgentInputSessionState'
import { docToContent, type InputModel } from './message-input/input-model'

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
  thinkingConfig,
  contextWindow,
  inputLimit,
  isRunning,
  isCompacting,
  canCompact,
  usage,
} = useAgentInputSessionState(workspace, props.conversationId)

function buildSubmitPayload(): UserContentBlock[] | null {
  const currentEditor = editor.value
  if (!currentEditor || currentEditor.isEmpty) return null
  const content = docToContent(currentEditor.getJSON() as InputModel)
  return content.length > 0 ? content : null
}

function clearInput(): void {
  editor.value?.commands.clearContent()
}

const { handleSubmit, handleSteerSubmit, handleQueueSubmit, handleSelectModel, handleChangeThinking, handleAbort, handleCompact } = useAgentInputActions({
  workspace,
  conversationId: props.conversationId,
  buildSubmitPayload,
  clearInput,
  emitEmptySubmit() {
    emit('empty-submit')
  },
})

const { editor, isFocused, hasContent } = useAgentInputEditor({
  handleSubmit,
  handleCancel() {},
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
    <div
      class="input-float rounded-xl outline outline-1 transition-[outline-color] duration-200"
      :class="isFocused ? 'bg-surface-raised outline-line-focus' : 'bg-surface-raised outline-line'"
    >
      <EditorContent v-if="editor" :editor="editor" />
      <div class="flex items-center justify-between px-3 pb-3">
        <div class="flex items-center gap-1">
          <ModelSelector
            :providers="workspace.providers.value"
            :models="workspace.models"
            :selected-provider-id="selectedProviderId"
            :selected-model-id="selectedModelId"
            v-bind="thinkingConfig ? { thinkingConfig } : {}"
            @select-model="handleSelectModel"
            @change-thinking="handleChangeThinking"
          />
        </div>
        <div class="flex items-center gap-1">
          <ContextUsageIndicator
            :conversation-id="props.conversationId"
            :usage="usage"
            :context-window="contextWindow"
            :input-limit="inputLimit"
            :is-compacting="isCompacting"
            :is-clickable="!isRunning && canCompact"
            @compact="handleCompact"
          />
          <template v-if="hasContent">
            <template v-if="isRunning">
              <Tooltip content="Steer current turn">
                <span
                  class="flex size-7 cursor-pointer items-center justify-center rounded-lg bg-overlay/10 text-fg transition-colors hover:bg-active"
                  @click="handleSteerSubmit"
                >
                  <SendLine :size="15" />
                </span>
              </Tooltip>
              <Tooltip content="Queue next turn">
                <span
                  class="flex size-7 cursor-pointer items-center justify-center rounded-lg bg-overlay/10 text-fg transition-colors hover:bg-active"
                  @click="handleQueueSubmit"
                >
                  <AddLine :size="15" />
                </span>
              </Tooltip>
            </template>
            <Tooltip v-else content="Send message">
              <span
                class="flex size-7 cursor-pointer items-center justify-center rounded-lg bg-overlay/10 text-fg transition-colors hover:bg-active"
                @click="handleSubmit"
              >
                <SendLine :size="15" />
              </span>
            </Tooltip>
          </template>
          <span
            v-else-if="isRunning || isCompacting"
            class="flex size-7 cursor-pointer items-center justify-center rounded-lg bg-overlay/10 text-fg transition-colors hover:bg-active"
            @click="handleAbort"
          >
            <StopLine :size="15" />
          </span>
          <span
            v-else
            class="flex size-7 items-center justify-center rounded-lg text-fg-faint"
          >
            <SendLine :size="15" />
          </span>
        </div>
      </div>
    </div>
  </div>
</template>

<style>
.input-float {
  box-shadow: var(--shadow-float);
}

.tiptap p.is-editor-empty:first-child::before {
  content: attr(data-placeholder);
  float: left;
  color: var(--color-fg-subtle);
  pointer-events: none;
  height: 0;
}
</style>
