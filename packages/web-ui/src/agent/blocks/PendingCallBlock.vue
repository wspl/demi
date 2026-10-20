<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { PendingCallRenderBlock } from '../pending-steers'
import FunctionalBlock from './FunctionalBlock.vue'
import { toolRowIcon } from './tool-row-icon'

/**
 * A call the model is still writing: its tool's row, shimmering, titled by
 * its description, which shows it from the moment the description is
 * written. The call's own block takes its place once it is whole.
 */
const props = defineProps<{ step: PendingCallRenderBlock }>()

const icon = computed(() => toolRowIcon(props.step.call.toolName))
</script>

<template>
  <FunctionalBlock loading>
    <template #icon>
      <component :is="icon" :size="ICON_PX.in28" />
    </template>
    <span class="min-w-0 truncate">{{ step.call.description }}</span>
  </FunctionalBlock>
</template>
