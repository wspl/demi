<script setup lang="ts">
import { CircleCheck, CircleX, X } from '@lucide/vue'
import { t } from '../infra/i18n'
import type { ToastTone } from '../infra/toast'
import { ICON_PX } from './icon-metrics'
import IconButton from './IconButton.vue'

withDefaults(defineProps<{
  title: string
  message?: string
  tone?: ToastTone
}>(), {
  tone: 'neutral',
})

const emit = defineEmits<{
  dismiss: []
}>()
</script>

<template>
  <div
    role="status"
    class="overlay-shell flex w-full items-start gap-2 rounded-lg py-2 pr-2 pl-3"
  >
    <span
      class="flex size-5 shrink-0 items-center justify-center"
      :class="tone === 'danger' ? 'text-on-danger' : 'text-on-success'"
    >
      <CircleX v-if="tone === 'danger'" :size="ICON_PX.in28" />
      <CircleCheck v-else :size="ICON_PX.in28" />
    </span>
    <div class="min-w-0 flex-1">
      <div class="flex h-5 items-center text-chrome leading-5 text-fg-emphasis">{{ title }}</div>
      <div
        v-if="message"
        class="text-[12px] leading-4 text-fg-muted"
      >{{ message }}</div>
    </div>
    <IconButton
      :icon="X"
      size="xs"
      variant="ghost"
      :aria-label="t('common.close')"
      @click="emit('dismiss')"
    />
  </div>
</template>
