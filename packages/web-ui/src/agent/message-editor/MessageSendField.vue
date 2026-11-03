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
 * (`MessageField.vue`), with the composer's send button at its end, such as
 * the follow-up a side question's card takes. Files come by paste and drop,
 * without an Add Files button, so its text starts where the pill's curve
 * allows.
 *
 * - Return sends, Shift-Return starts a new line; Escape does nothing, as in
 *   the composer.
 * - It grows with its message up to a few lines.
 * - While it cannot send, it still takes what is typed, and its send button
 *   and Return send nothing.
 */
const props = withDefaults(defineProps<{
  files: MessageFiles
  placeholder: PlaceholderText
  /** The send button's name, such as Reply. */
  sendLabel: TitleText
  /** It may send now; false while what it would send to is busy. */
  canSend?: boolean
}>(), { canSend: true })

const message = defineModel<DraftMessage>({ default: () => ({ text: '', fileIds: [] }) })

const emit = defineEmits<{
  send: [message: DraftMessage]
}>()

const field = ref<InstanceType<typeof MessageField>>()
const empty = computed(() => !saysSomething(message.value))
const disabled = computed(() => empty.value || !props.canSend)

function send(): void {
  if (disabled.value)
    return
  emit('send', { text: message.value.text.trim(), fileIds: [...message.value.fileIds] })
  message.value = { text: '', fileIds: [] }
}

defineExpose({ focus: () => field.value?.focus() })
</script>

<template>
  <div class="flex items-end gap-1 rounded-[18px] bg-surface-raised py-1 pl-3 pr-1 ring-1 ring-line focus-within:ring-line-focus">
    <!--
      The text starts clear of the pill's round end; the send button sits
      close to the other, as the composer's does.
    -->
    <div class="max-h-32 min-h-7 min-w-0 flex-1 overflow-y-auto py-1 text-chrome leading-5 text-fg">
      <MessageField
        ref="field"
        v-model="message"
        :files="files"
        :placeholder="placeholder"
        @submit="send"
      />
    </div>
    <Tooltip :content="sendLabel">
      <IconButton
        :icon="ArrowUp"
        :variant="disabled ? 'ghost' : 'accent'"
        circle
        :disabled="disabled"
        :aria-label="sendLabel"
        @mousedown.prevent
        @click="send"
      />
    </Tooltip>
  </div>
</template>
