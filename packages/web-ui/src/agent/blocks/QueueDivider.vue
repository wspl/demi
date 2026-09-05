<script setup lang="ts">
import { t } from '../../infra/i18n'

defineProps<{
  count: number
}>()
</script>

<template>
  <div class="flex items-center gap-3 px-[var(--agent-pad-x,2rem)]">
    <div class="h-px flex-1 bg-line-subtle" />
    <span class="text-chrome text-fg-subtle">{{ count }} {{ t('agent.queue.queued') }}</span>
    <div class="h-px flex-1 bg-line-subtle" />
  </div>
</template>
