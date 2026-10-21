<script setup lang="ts">
import { ArrowDown } from '@lucide/vue'
import { computed } from 'vue'
import { usePreferredReducedMotion } from '@vueuse/core'
import type { RequestOutputReading } from '@demicodes/conversation-client'
import type { TurnSpeed } from '@demicodes/protocol'
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
  /** The speed of the conversation's last turn, as the server keeps it. */
  turnSpeed: TurnSpeed | null
}>()

const motion = usePreferredReducedMotion()
const { shown, count, speed, turnSpeed } = useRequestOutput({
  output: () => props.output(),
  scope: () => props.scope,
  turnSpeed: () => props.turnSpeed,
  reducedMotion: () => motion.value === 'reduce',
})

const FADE = { leaveActiveClass: 'transition-opacity duration-200 ease-out', leaveToClass: 'opacity-0' }

/** The speed the row shows: the reading's while one shows, then the last turn's. */
const rate = computed(() => (shown.value ? speed.value : turnSpeed.value))
</script>

<template>
  <!--
    The reading's parts wrap onto a second line the row clips, the speed
    first, then the count, so a part never shows cut. The empty first part
    lets the count wrap away too, and the last turn's speed alone. The row's
    gap, not the separator, keeps the parts apart: a count that fades out
    beside the last turn's speed never had a separator when no speed was
    read yet.
  -->
  <Transition v-bind="FADE">
    <div
      v-if="shown || turnSpeed !== null"
      class="flex h-chip min-w-0 flex-1 flex-wrap content-start items-center justify-end gap-x-1 overflow-hidden text-detail text-fg-subtle tabular-nums"
      data-request-output
    >
      <span class="h-chip" aria-hidden="true" />
      <Transition v-bind="FADE">
        <span v-if="shown" class="inline-flex h-chip shrink-0 items-center gap-1 whitespace-nowrap">
          <ArrowDown :size="ICON_PX.in20" />
          {{ t('dock.outputTokens', { tokens: formatTokens(Math.round(count)) }, Math.round(count)) }}
        </span>
      </Transition>
      <!--
        One element for both speeds and the separator, so the speed keeps its
        place while the count and its separator fade, and wraps with its
        separator.
      -->
      <span
        v-if="rate !== null"
        class="inline-flex h-chip shrink-0 items-center whitespace-nowrap"
        :class="shown ? '' : 'text-fg-faint'"
        :data-turn-speed="shown ? undefined : ''"
      >
        <Transition v-bind="FADE">
          <span v-if="shown">·&nbsp;</span>
        </Transition>
        {{ t(shown ? 'dock.tokenRate' : 'dock.lastTurnRate', { rate: formatDecimal(rate, 0) }) }}
      </span>
    </div>
  </Transition>
</template>
