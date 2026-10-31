<script setup lang="ts">
import { computed, ref } from 'vue'
import Button from '../../ui/Button.vue'
import MessageField from '../message-editor/MessageField.vue'
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'
import { saysSomething, type DraftMessage, type MessageFiles } from './comments'

/**
 * A waiting comment being edited: its message in the composer's editor
 * (`MessageField.vue`), with Add Files, Save and Cancel. Return saves,
 * Shift-Return starts a new line, Escape cancels.
 */
const props = defineProps<{
  message: DraftMessage
  files: MessageFiles
}>()

const emit = defineEmits<{
  save: [message: DraftMessage]
  cancel: []
}>()

const draft = ref<DraftMessage>({ text: props.message.text, fileIds: [...props.message.fileIds] })
const empty = computed(() => !saysSomething(draft.value))

function save(): void {
  if (!empty.value)
    emit('save', { text: draft.value.text.trim(), fileIds: [...draft.value.fileIds] })
}
</script>

<template>
  <div class="flex flex-col gap-2">
    <div class="max-h-48 overflow-y-auto rounded-lg bg-surface-raised px-3 py-1.5 ring-1 ring-line focus-within:ring-line-focus">
      <!-- Edit asked for the field: it takes the focus, on a phone too, the caret at the end. -->
      <MessageField
        v-model="draft"
        :files="files"
        autofocus
        cancelable
        @submit="save"
        @cancel="emit('cancel')"
      />
    </div>
    <div class="flex items-center gap-2">
      <span class="flex-1" />
      <Button size="sm" @click="emit('cancel')">{{ shared('common.cancel') }}</Button>
      <Button size="sm" variant="primary" :disabled="empty" :disabled-reason="t('comments.writeFirst')" @click="save">{{ shared('common.save') }}</Button>
    </div>
  </div>
</template>
