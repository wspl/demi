<script setup lang="ts">
import { computed } from 'vue'
import type { PendingCall } from '@demicodes/protocol'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { pendingCallTitle } from '../tool-rendering'
import FunctionalBlock from './FunctionalBlock.vue'
import { toolRowIcon } from './tool-row-icon'

/**
 * A call the model is still writing (`runtime.md` § Calls being written):
 * its tool's row, shimmering, titled by its description once written and by
 * its tool until then. The call's own block takes its place once it is whole.
 */
const props = defineProps<{ call: PendingCall }>()

const icon = computed(() => toolRowIcon(props.call.toolName))
const title = computed(() => pendingCallTitle(props.call))
</script>

<template>
  <FunctionalBlock loading>
    <template #icon>
      <component :is="icon" :size="ICON_PX.in28" />
    </template>
    <span class="min-w-0 truncate">{{ title }}</span>
  </FunctionalBlock>
</template>
