<script setup lang="ts">
import { t } from '@demi/web-ui/infra/i18n'

defineProps<{
  status: 'error'
}>()
</script>

<template>
  <span class="rounded bg-tint-danger-strong px-1.5 py-1 text-[10px] font-medium uppercase tracking-wide text-on-danger">
    {{ t('agent.block.error') }}
  </span>
</template>
