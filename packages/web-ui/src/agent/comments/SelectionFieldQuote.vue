<script setup lang="ts">
import { X } from '@lucide/vue'
import IconButton from '../../ui/IconButton.vue'
import { t as shared } from '../../locales/text'
import CommentQuote from './CommentQuote.vue'

/**
 * The first row of the phone's comment sheet: the selection quoted in one line,
 * then the field's ✕ at the row's end, which closes the field (`close`). The
 * phone's field itself has no ✕.
 */
defineProps<{
  /** The selection's words as the row quotes them; none leaves the ✕ alone. */
  quote?: string
}>()

const emit = defineEmits<{
  close: []
}>()
</script>

<template>
  <div class="mb-1 ml-1 flex items-center gap-1">
    <CommentQuote v-if="quote" :text="quote" :lines="1" class="min-w-0 flex-1" />
    <IconButton
      :icon="X"
      size="sm"
      variant="ghost"
      class="ml-auto"
      :aria-label="shared('common.close')"
      @mousedown.prevent
      @click="emit('close')"
    />
  </div>
</template>
