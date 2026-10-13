<script setup lang="ts">
import type { Component } from 'vue'
import { CircleCheck, CircleX, Info, X } from '@lucide/vue'
import type { ToastTone } from '../infra/toast'
import { ICON_PX } from './icon-metrics'
import Button from './Button.vue'
import IconButton from './IconButton.vue'
import RequestIdLine from './RequestIdLine.vue'
import type { HeadlineText, SentenceText, TitleText } from './ui-text'

defineProps<{
  title: HeadlineText
  message?: SentenceText
  tone: ToastTone
  /** The label of its one action, which emits `act`. */
  action?: TitleText
  /** The request a failure came from, named under the message. */
  requestId?: string | null
}>()

/** Each tone's mark: a check for success, an i for a fact, a cross for a failure. */
const MARKS: Record<ToastTone, { icon: Component; color: string }> = {
  success: { icon: CircleCheck, color: 'text-on-success' },
  neutral: { icon: Info, color: 'text-fg-muted' },
  danger: { icon: CircleX, color: 'text-on-danger' },
}

const emit = defineEmits<{
  dismiss: []
  act: []
}>()
</script>

<template>
  <div
    role="status"
    class="overlay-shell flex w-full items-start gap-2 rounded-lg py-2 pr-2 pl-3"
  >
    <span
      class="flex size-5 shrink-0 items-center justify-center"
      :class="MARKS[tone].color"
    >
      <component :is="MARKS[tone].icon" :size="ICON_PX.in28" />
    </span>
    <div class="min-w-0 flex-1">
      <!-- The title wraps; 20px lines keep a one-line title centred on the icon. -->
      <div class="min-h-5 text-chrome leading-5 text-fg-emphasis">{{ title }}</div>
      <!-- A message of several lines, such as one per refused conversation, keeps its lines. -->
      <div
        v-if="message"
        class="text-[12px] leading-4 whitespace-pre-line text-fg-muted"
      >{{ message }}</div>
      <RequestIdLine v-if="requestId" class="text-fg-subtle" :request-id="requestId" />
      <Button
        v-if="action"
        class="mt-2"
        size="sm"
        @click="emit('act')"
      >{{ action }}</Button>
    </div>
    <IconButton
      :icon="X"
      size="xs"
      variant="ghost"
      aria-label="Close"
      @click="emit('dismiss')"
    />
  </div>
</template>
