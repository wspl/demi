<script setup lang="ts">
import { computed } from 'vue'
import { pageLanguage } from '../i18n/text'
import { t as shared } from '../locales/text'
import Button from '../ui/Button.vue'
import Fold from '../ui/Fold.vue'
import FoldChevron from '../ui/FoldChevron.vue'
import { ICON_PX } from '../ui/icon-metrics'
import RequestIdLine from '../ui/RequestIdLine.vue'
import SectionRow from '../ui/SectionRow.vue'
import { formatDollars } from '../ui/numbers'
import { formatTokens } from '../ui/token-count'
import CacheHitChart from './CacheHitChart.vue'
import {
  type CostState,
  CHART_CALLS,
  hitRate,
  profileRows,
  recentCalls,
} from './context-cost'
import { t } from './locales/text'

/**
 * The context card's Cost section
 * (`packages/web-ui/src/agent/ContextUsageIndicator.vue`): closed, the
 * equivalent API price and the main conversation's cache
 * hit rate side by side; opened by its caption, a disclosure of its own,
 * the price by kind of token, the share of the profiles the conversation's
 * workflow offers and the chart of the main conversation's recent cache
 * hits. A figure that cannot be computed is left out with its label. A read
 * that failed says so under the caption, with its request's id and Retry,
 * and has nothing to open.
 */
const props = defineProps<{
  state: CostState
  /** Whether the section stands opened. */
  open: boolean
  /** The section is the card's first, with no divider above it. */
  first?: boolean
  /** The call, by its place among the conversation's, whose tip the chart shows without the pointer, as a gallery specimen pins it. */
  pinnedTip?: number | null
  /**
   * The names of the profiles the conversation's workflow offers now, whose
   * subagents' prices the Profiles part shows; none under No Subagents.
   */
  profiles?: readonly string[]
}>()
const emit = defineEmits<{
  /** Read the cost again after a read that failed. */
  retry: []
  /** The caption was clicked, which opens or closes the section. */
  toggle: []
}>()

/** The cost the section shows; none while it shows a failed read. */
const cost = computed(() => (props.state.status === 'read' ? props.state.cost : null))

const price = computed(() => cost.value?.totals.price.total ?? null)
/** The main conversation's recent calls, which the rate counts and the chart draws. */
const recent = computed(() => recentCalls(cost.value?.calls ?? []))
const rate = computed(() => (cost.value ? hitRate(recent.value.calls) : null))
const percent = new Intl.NumberFormat(pageLanguage(), { style: 'percent', maximumFractionDigits: 0 })
/** The chart shows once the main conversation made enough calls to draw a line. */
const charted = computed(() => recent.value.calls.length >= CHART_CALLS)

/** The two figures of the closed section, each that can be computed. */
const figures = computed(() => [
  ...(price.value === null ? [] : [{ key: 'price', value: formatDollars(price.value), label: t('context.cost.price') }]),
  ...(rate.value === null ? [] : [{ key: 'rate', value: percent.format(rate.value), label: t('context.cost.hitRate') }]),
])

const kinds = computed(() => {
  if (!cost.value)
    return []
  const { totals } = cost.value
  return [
    { key: 'input', label: t('context.cost.kind.input'), tokens: totals.inputTokens, price: totals.price.input },
    { key: 'cacheWrite', label: t('context.cost.kind.cacheWrite'), tokens: totals.cacheWriteTokens, price: totals.price.cacheWrite },
    { key: 'cacheRead', label: t('context.cost.kind.cacheRead'), tokens: totals.cacheReadTokens, price: totals.price.cacheRead },
    { key: 'output', label: t('context.cost.kind.output'), tokens: totals.outputTokens, price: totals.price.output },
  ]
})

const rows = computed(() => profileRows(cost.value?.agents ?? [], props.profiles ?? []))
const largest = computed(() => Math.max(0, ...rows.value.map((row) => row.price)))
function share(price: number): string {
  return `${largest.value > 0 ? (price / largest.value) * 100 : 0}%`
}
</script>

<template>
  <div :class="[first ? 'pt-1' : 'border-t border-line', cost && open ? 'pb-2' : 'pb-1']">
    <!-- The caption is the section's disclosure, as the context window's header is. -->
    <button
      v-if="cost"
      type="button"
      :aria-expanded="open"
      class="block w-full cursor-default text-left"
      @click="emit('toggle')"
    >
      <SectionRow class="text-caption font-medium text-fg-subtle">
        <span class="shrink-0">{{ t('context.cost.title') }}</span>
        <template #end>
          <FoldChevron :open="open" :size="ICON_PX.in20" />
        </template>
      </SectionRow>
    </button>
    <SectionRow v-else class="text-caption font-medium text-fg-subtle">
      <span class="shrink-0">{{ t('context.cost.title') }}</span>
    </SectionRow>
    <div v-if="state.status === 'failed'" class="px-3 pb-2 text-detail leading-4 text-fg-subtle">
      <div class="flex items-center gap-3">
        <p class="min-w-0 flex-1 break-words">{{ t('context.cost.loadFailed') }}</p>
        <Button size="sm" @click="emit('retry')">{{ shared('common.retry') }}</Button>
      </div>
      <RequestIdLine v-if="state.error.requestId" :request-id="state.error.requestId" />
    </div>
    <!-- Closed, two figures side by side: the price at the text's left edge, the rate at the middle. -->
    <Fold v-if="cost" :open="!open">
      <div class="cost-figures px-3 pb-1">
        <div v-for="figure in figures" :key="figure.key">
          <div class="cost-figure whitespace-nowrap tabular-nums">{{ figure.value }}</div>
          <div class="cost-figure-label text-fg-subtle">{{ figure.label }}</div>
        </div>
      </div>
    </Fold>
    <Fold v-if="cost" :open="open">
      <div v-if="price !== null" class="flex min-h-6 items-center gap-3 px-3">
        <span class="min-w-0 flex-1 truncate text-fg-body">{{ t('context.cost.price') }}</span>
        <span class="whitespace-nowrap font-medium tabular-nums text-fg-emphasis">{{ formatDollars(price) }}</span>
      </div>
      <ul class="cost-rows px-3">
        <li v-for="kind in kinds" :key="kind.key" class="cost-row">
          <span class="min-w-0 truncate text-fg-body">{{ kind.label }}</span>
          <span class="whitespace-nowrap text-right tabular-nums text-fg-subtle">{{ formatTokens(kind.tokens) }}</span>
          <span class="whitespace-nowrap text-right tabular-nums text-fg-subtle">{{ kind.price === null ? '' : formatDollars(kind.price) }}</span>
        </li>
      </ul>
      <template v-if="rows.length">
        <div class="cost-caption px-3">
          <span class="text-caption font-medium text-fg-subtle">{{ t('context.cost.profiles') }}</span>
        </div>
        <ul class="px-3">
          <li v-for="row in rows" :key="row.key" class="flex flex-col gap-[3px] pt-1 pb-[3px]">
            <div class="flex items-baseline gap-3">
              <span class="min-w-0 truncate text-fg-muted">{{ row.label }}</span>
              <span class="ml-auto whitespace-nowrap tabular-nums text-fg-subtle">{{ formatDollars(row.price) }}</span>
            </div>
            <div class="h-[2px] overflow-hidden rounded-full bg-overlay/5" aria-hidden="true">
              <div class="h-full rounded-full bg-fg-subtle" :style="{ width: share(row.price) }" />
            </div>
          </li>
        </ul>
      </template>
      <template v-if="cost && rate !== null">
        <div class="cost-caption px-3">
          <span class="text-caption font-medium">
            <span class="text-fg-subtle">{{ t('context.cost.hitRate') }}</span>
            <span class="ml-1.5 tabular-nums text-fg-emphasis">{{ percent.format(rate) }}</span>
          </span>
        </div>
        <div v-if="charted" class="px-3">
          <CacheHitChart :calls="recent.calls" :before="recent.before" :pinned-tip="pinnedTip" />
        </div>
      </template>
    </Fold>
  </div>
</template>

<style scoped>
/* Two figures side by side, each its value over its label. */
.cost-figures {
  display: grid;
  grid-template-columns: 1fr 1fr;
  column-gap: 0.75rem;
}

.cost-figure {
  font-size: 15px;
  line-height: 1.25rem;
  font-weight: 500;
  color: var(--color-fg-emphasis);
}

.cost-figure-label {
  font-size: var(--text-caption);
  line-height: 0.875rem;
}

/* The kinds of token: their names, their tokens and their prices, each
   figure's column as wide as its widest figure across the rows, so a price
   never wraps and the figures line up. */
.cost-rows {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  column-gap: 0.5rem;
}

.cost-row {
  display: grid;
  grid-column: 1 / -1;
  grid-template-columns: subgrid;
  align-items: center;
  min-height: 1.5rem;
}

/* A part's caption. */
.cost-caption {
  display: flex;
  align-items: center;
  min-height: 1.75rem;
  padding-top: 0.5rem;
  padding-bottom: 0.125rem;
}
</style>
