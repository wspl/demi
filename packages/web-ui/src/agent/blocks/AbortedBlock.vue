<script setup lang="ts">
import { CircleStop } from '@lucide/vue'
import { t } from '@demicodes/web-ui/infra/i18n'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'
</script>

<template>
  <FunctionalBlock>
    <template #icon>
      <CircleStop :size="ICON_PX.in28" />
    </template>
    <span class="min-w-0 truncate">{{ t('agent.block.aborted') }}</span>
  </FunctionalBlock>
</template>
