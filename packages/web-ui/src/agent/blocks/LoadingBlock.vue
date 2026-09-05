<script setup lang="ts">
import ActivityMark from '@demicodes/web-ui/ui/ActivityMark.vue'
import { t } from '@demicodes/web-ui/infra/i18n'
import FunctionalBlock from './FunctionalBlock.vue'

withDefaults(defineProps<{
  label?: string
}>(), {
  label: '',
})
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <FunctionalBlock loading :label="label || t('agent.block.requesting')">
      <template #icon>
        <ActivityMark />
      </template>
    </FunctionalBlock>
  </div>
</template>
