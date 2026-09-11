<script setup lang="ts">
import { CircleX, X } from '@lucide/vue'
import { t } from '../infra/i18n'
import Button from './Button.vue'
import IconButton from './IconButton.vue'

/**
 * A failed operation whose input is still on screen: a rejected form, a send
 * that did not go through, an upload that stopped, an edit the server refused.
 *
 * It sits directly under the control that failed, in that control's width, so
 * the reader sees the message next to what they can still change. It carries at
 * most one action (Retry, Review…) and an optional dismiss. Every such failure
 * in the product uses this component; no page draws its own red line.
 */
withDefaults(
  defineProps<{
    message: string
    /** The single action's label; the caller decides what it does. */
    action?: string
    actionPending?: boolean
    dismissible?: boolean
  }>(),
  {
    actionPending: false,
    dismissible: false,
  },
)

defineEmits<{
  action: []
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
    <Button
      v-if="action"
      size="xs"
      class="-my-0.5 shrink-0"
      :loading="actionPending"
      @click="$emit('action')"
      >{{ action }}</Button
    >
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
