<script setup lang="ts">
import { EyeOff } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import NoticeRow from './NoticeRow.vue'
import { t } from '../locales/text'

/**
 * Where a share holds a run of the agent's steps it does not show
 * (`sharing.md` § Steps not shared): one muted line that counts them, in
 * their place, standing as the transcript's other rows of work stand, in
 * the same inset as a work group's row, and does not open.
 */
defineProps<{
  count: number
}>()
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <NoticeRow>
      <template #icon>
        <EyeOff class="text-fg-faint" :size="ICON_PX.in28" />
      </template>
      <span class="truncate text-fg-subtle" data-steps-not-shared>{{ t('transcript.stepsNotShared', { count }, count) }}</span>
    </NoticeRow>
  </div>
</template>
