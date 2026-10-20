<script setup lang="ts">
import { ArrowDown } from '@lucide/vue'
import { usePreferredReducedMotion } from '@vueuse/core'
import type { RequestOutputReading } from '@demicodes/conversation-client'
import { ICON_PX } from '../ui/icon-metrics'
import { formatDecimal } from '../ui/numbers'
import { formatTokens } from '../ui/token-count'
import { t } from './locales/text'
import { useRequestOutput } from './request-output'

const props = defineProps<{
  /** Reads the server's reading of the request the root agent runs (`SessionDock`). */
  output: () => RequestOutputReading | null
  /** The conversation it belongs to. */
  scope: string
}>()

const motion = usePreferredReducedMotion()
const { shown, count, speed } = useRequestOutput({
  output: () => props.output(),
  scope: () => props.scope,
  reducedMotion: () => motion.value === 'reduce',
})
</script>

<template>
  <!--
    The reading's parts wrap onto a second line the row clips, the speed
    first, then the count, so a part never shows cut. The empty first part
    lets the count wrap away too.
  -->
  <Transition leave-active-class="transition-opacity duration-200 ease-out" leave-to-class="opacity-0">
    <div
      v-if="shown"
      class="flex h-chip min-w-0 flex-1 flex-wrap content-start items-center justify-end overflow-hidden text-chrome text-fg-subtle tabular-nums"
      data-request-output
    >
      <span class="h-chip" aria-hidden="true" />
      <span class="inline-flex h-chip shrink-0 items-center gap-1 whitespace-nowrap">
        <ArrowDown :size="ICON_PX.in28" />
        {{ t('dock.outputTokens', { tokens: formatTokens(Math.round(count)) }, Math.round(count)) }}
      </span>
      <span v-if="speed !== null" class="inline-flex h-chip shrink-0 items-center whitespace-nowrap">
        &nbsp;·&nbsp;{{ t('dock.tokenRate', { rate: formatDecimal(speed, 0) }) }}
      </span>
    </div>
  </Transition>
</template>
