<script setup lang="ts">
import { CircleX, X } from '@lucide/vue'
import IconButton from './IconButton.vue'
import { t as shared } from '../locales/text'
import RequestIdLine from './RequestIdLine.vue'
import type { SentenceText } from './ui-text'

/**
 * A form's own failure: a rejected sign-in, a wrong code, a value the server
 * refused, a folder that could not be created.
 *
 * It sits directly under the fields it is about, in their width, as a line of
 * text the reader corrects against. It never carries a retry; the form's own
 * submit is the retry.
 *
 * A failure that is not about the form's input (the request could not reach the
 * server, the save failed for a reason the reader cannot fix here) is not shown
 * inline. It goes to a toast through `reportError`.
 */
withDefaults(
  defineProps<{
    message: SentenceText
    /** The request the failure came from, named under the message. */
    requestId?: string | null
    dismissible?: boolean
  }>(),
  {
    requestId: null,
    dismissible: false,
  },
)

defineEmits<{
  dismiss: []
}>()
</script>

<template>
  <div
    role="alert"
    class="flex items-start gap-1.5 text-detail leading-4 text-on-danger"
  >
    <span class="flex h-4 shrink-0 items-center" aria-hidden="true">
      <CircleX :size="12" />
    </span>
    <div class="min-w-0">
      <p class="whitespace-pre-line break-words">{{ message }}</p>
      <RequestIdLine v-if="requestId" class="text-fg-muted" :request-id="requestId" />
    </div>
    <IconButton
      v-if="dismissible"
      :icon="X"
      size="xs"
      variant="ghost"
      class="-my-0.5 shrink-0"
      :aria-label="shared('common.close')"
      @click="$emit('dismiss')"
    />
  </div>
</template>
