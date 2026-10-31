<script setup lang="ts">
import { computed, ref } from 'vue'
import { ArrowUp } from '@lucide/vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import type { PlaceholderText, TitleText } from '../../ui/ui-text'
import { saysSomething, type DraftMessage, type MessageFiles } from '../comments/comments'
import { t } from '../locales/text'
import MessageField from './MessageField.vue'

/**
 * A small field that sends a message, written as the composer's is
 * (`MessageField.vue`), with the Add Files button at its start and the
 * composer's send button at its end, such as the follow-up a side question's
 * card takes.
 *
 * - Return sends, Shift-Return starts a new line, Escape cancels.
 * - It grows with its message up to a few lines.
 */
defineProps<{
  files: MessageFiles
  placeholder: PlaceholderText
  /** The send button's name, such as Reply. */
  sendLabel: TitleText
  /** It takes the focus as it opens, on a phone too. */
  focus?: boolean
}>()

const message = defineModel<DraftMessage>({ default: () => ({ text: '', fileIds: [] }) })

const emit = defineEmits<{
  send: [message: DraftMessage]
  cancel: []
}>()

const field = ref<InstanceType<typeof MessageField>>()
const empty = computed(() => !saysSomething(message.value))

function send(): void {
  if (empty.value)
    return
  emit('send', { text: message.value.text.trim(), fileIds: [...message.value.fileIds] })
  message.value = { text: '', fileIds: [] }
}

defineExpose({ focus: () => field.value?.focus() })
</script>

<template>
  <div class="flex items-end gap-1 rounded-[18px] bg-surface-raised py-1 pl-1 pr-1 ring-1 ring-line focus-within:ring-line-focus">
    <div class="max-h-32 min-h-7 min-w-0 flex-1 overflow-y-auto py-1 text-chrome leading-5 text-fg">
      <MessageField
        ref="field"
        v-model="message"
        :files="files"
        :placeholder="placeholder"
        :autofocus="focus"
        cancelable
        @submit="send"
        @cancel="emit('cancel')"
      />
    </div>
    <Tooltip :content="sendLabel">
      <IconButton
        :icon="ArrowUp"
        :variant="empty ? 'ghost' : 'accent'"
        circle
        :disabled="empty"
        :aria-label="sendLabel"
        @mousedown.prevent
        @click="send"
      />
    </Tooltip>
  </div>
</template>
