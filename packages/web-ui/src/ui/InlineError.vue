<script setup lang="ts">
import { CircleX, X } from '@lucide/vue'
import { t } from '../infra/i18n'
import IconButton from './IconButton.vue'

/**
 * A form's own failure: a rejected sign-in, a wrong code, a value the server
 * refused, a folder that could not be created. It sits directly under the
 * fields it is about, in their width, as a line of text the reader corrects
 * against. It never carries a retry; the form's own submit is the retry.
 *
 * A failure that is not about the form's input (the request could not reach
 * the server, the save failed for a reason the reader cannot fix here) is not
 * shown inline: it goes to a toast through `reportError`.
 */
withDefaults(
  defineProps<{
    message: string
    dismissible?: boolean
  }>(),
  {
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
    class="flex items-start gap-1.5 text-[12px] leading-4 text-on-danger"
  >
    <span class="flex h-4 shrink-0 items-center" aria-hidden="true">
      <CircleX :size="12" />
    </span>
    <p class="min-w-0 whitespace-pre-line break-words">{{ message }}</p>
    <IconButton
      v-if="dismissible"
      :icon="X"
      size="xs"
      variant="ghost"
      class="-my-0.5 shrink-0"
      :aria-label="t('common.close')"
      @click="$emit('dismiss')"
    />
  </div>
</template>
