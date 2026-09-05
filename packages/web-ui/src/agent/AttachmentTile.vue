<script setup lang="ts">
import { File, X } from '@lucide/vue'
import { t } from '../infra/i18n'
import { ICON_PX } from '../ui/icon-metrics'

defineProps<{
  name: string
  src?: string
  removable?: boolean
}>()

const emit = defineEmits<{
  remove: []
}>()
</script>

<template>
  <span class="relative block size-12" :title="name">
    <img
      v-if="src"
      :src="src"
      :alt="name"
      class="size-12 rounded-lg object-cover ring-1 ring-line"
    >
    <span
      v-else
      class="flex size-12 items-center justify-center rounded-lg bg-surface text-fg-muted ring-1 ring-line"
    >
      <File :size="18" />
    </span>
    <button
      v-if="removable"
      type="button"
      class="absolute top-px right-px flex size-3 items-center justify-center rounded-full bg-surface-raised text-fg-muted ring-1 ring-line hover:text-fg-body"
      :aria-label="t('common.close')"
      @click.stop="emit('remove')"
    >
      <X :size="ICON_PX.in12" />
    </button>
  </span>
</template>
