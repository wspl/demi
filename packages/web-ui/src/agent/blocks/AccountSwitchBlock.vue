<script setup lang="ts">
import { computed } from 'vue'
import { ArrowRightLeft } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { useClock } from '../../composables/useClock'
import { accountSwitchText } from '../account-switch'
import type { AccountSwitchBlock } from '../block-types'

/**
 * Where the agent's account reached its limit and the same request went on
 * with another account: a neutral line, as the Stopped row is, not a
 * failure (`crates/backend-providers/src/llm/rotation.rs`). It is the whole of what
 * happened, so it wraps where the transcript is narrow rather than cut
 * the account it went on with.
 */
const props = defineProps<{
  block: Pick<AccountSwitchBlock, 'from' | 'to' | 'window' | 'resetsAt'>
}>()

const now = useClock()
const text = computed(() => accountSwitchText(props.block, new Date(now.value)))
</script>

<template>
  <div class="flex items-start gap-2 text-row text-fg-row">
    <div
      class="flex shrink-0 items-center justify-center"
      :style="{ width: `${ICON_PX.in28}px`, height: '1.75rem' }"
    >
      <ArrowRightLeft :size="ICON_PX.in28" />
    </div>
    <p class="min-w-0 break-words py-1 leading-5">{{ text }}</p>
  </div>
</template>
