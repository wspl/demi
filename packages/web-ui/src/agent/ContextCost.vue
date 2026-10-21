<script setup lang="ts">
import { computed } from 'vue'
import { pageLanguage } from '../i18n/text'
import Fold from '../ui/Fold.vue'
import FoldChevron from '../ui/FoldChevron.vue'
import { ICON_PX } from '../ui/icon-metrics'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import SectionRow from '../ui/SectionRow.vue'
import { formatDollars } from '../ui/numbers'
import CacheHitChart from './CacheHitChart.vue'
import { type CostRange, type CostShown, lastHourOffered, profileRows } from './context-cost'
import { t } from './locales/text'

/**
 * The context card's Cost section
 * (`packages/web-ui/src/agent/ContextUsageIndicator.vue`).
 *
 * Closed, the section shows the equivalent API price and the main
 * conversation's cache hit rate side by side. Its caption is a disclosure of
 * its own. Opened, its caption holds the price, and the section shows:
 *
 * - the price by kind of token, two by two
 * - the share of the profiles the conversation's workflow offers, in two
 *   columns
 * - the chart of the agent's recent cache hits
 *
 * The rate and the chart are the backend's, which come with the agent's
 * node (`docs/decisions/0019-the-server-counts-the-cache-chart.md`), so the
 * section opens with them. A figure the section does not have is left out
 * with its label: the rate of an agent that sent no prompt yet, and the
 * chart before its fourth call.
 *
 * Once the conversation's first call is more than an hour old, a switch in
 * the caption, before its chevron, counts the last hour instead of the whole
 * conversation: the prices, the profiles' shares, the rate and the closed
 * section's two figures; the chart stays, with the rate's label alone when
 * the hour has no rate
 * (`docs/decisions/0041-the-cost-shows-the-last-hour.md`). The last hour's
 * figures are left out until the backend's first count of them arrives.
 */
const props = defineProps<{
  state: CostShown
  /** Whether the section stands opened. */
  open: boolean
  /** The section is the card's first, with no divider above it. */
  first?: boolean
  /**
   * The call, by its place among the conversation's, whose tip the chart shows
   * without the pointer, as a gallery specimen pins it.
   */
  pinnedTip?: number | null
  /**
   * The names of the profiles the conversation's workflow offers now. The
   * Profiles part shows their subagents' prices. None under No Subagents.
   */
  profiles?: readonly string[]
}>()
const emit = defineEmits<{
  /** The caption was clicked, which opens or closes the section. */
  toggle: []
}>()

/** The range the section counts; the whole conversation unless the parent says. */
const range = defineModel<CostRange>('range', { default: 'all' })
const rangeOptions = computed<readonly SegmentedOption<CostRange>[]>(() => [
  { value: 'lastHour', label: t('context.cost.range.lastHour') },
  { value: 'all', label: t('context.cost.range.all') },
])
const offered = computed(() => lastHourOffered(props.state.firstCallAt))
const counting = computed(() => offered.value && range.value === 'lastHour')

/** What the section counts now: null while the last hour's count is awaited. */
const counted = computed(() => (counting.value
  ? props.state.lastHour
  : { cost: props.state.cost, rate: props.state.chart?.rate ?? null }))
const cost = computed(() => counted.value?.cost ?? null)
const chart = computed(() => props.state.chart)

// An hour without a priced call costs nothing, where the whole conversation
// that has no priced call has no price.
const price = computed(() => cost.value?.totals.price.total
  ?? (counting.value && cost.value && props.state.cost.totals.price.total !== null ? 0 : null))
const rate = computed(() => counted.value?.rate ?? null)
// The chart is the agent's latest calls whatever the range counts, so it
// shows when the last hour has no rate: an hour of no call of the agent's, or
// a count still awaited.
const charted = computed(() => (chart.value?.points.length ?? 0) > 0)
const percent = new Intl.NumberFormat(pageLanguage(), { style: 'percent', maximumFractionDigits: 0 })

/** The two figures of the closed section, each that can be computed. */
const figures = computed(() => [
  ...(price.value === null ? [] : [{ key: 'price', value: formatDollars(price.value), label: t('context.cost.price') }]),
  ...(rate.value === null ? [] : [{ key: 'rate', value: percent.format(rate.value), label: t('context.cost.hitRate') }]),
])

const kinds = computed(() => {
  if (!cost.value)
    return []
  const { price: prices } = cost.value.totals
  // Row by row: input and output, then the cache's write and read; a kind
  // with no price is left out with its label.
  return [
    { key: 'input', label: t('context.cost.kind.input'), price: prices.input },
    { key: 'output', label: t('context.cost.kind.output'), price: prices.output },
    { key: 'cacheWrite', label: t('context.cost.kind.cacheWrite'), price: prices.cacheWrite },
    { key: 'cacheRead', label: t('context.cost.kind.cacheRead'), price: prices.cacheRead },
  ].flatMap(({ price, ...kind }) => (price === null ? [] : [{ ...kind, price }]))
})

const rows = computed(() => profileRows(cost.value?.agents ?? [], props.profiles ?? []))
const largest = computed(() => Math.max(0, ...rows.value.map((row) => row.price)))
function share(price: number): string {
  return `${largest.value > 0 ? (price / largest.value) * 100 : 0}%`
}
</script>

<template>
  <div :class="[first ? 'pt-1' : 'border-t border-line', open ? 'pb-2' : 'pb-1']">
    <!--
      The caption is the section's disclosure, as the context window's header
      is: the whole row opens and closes it, the switch apart, and its title
      is the button a keyboard reaches.
    -->
    <SectionRow class="cursor-default text-caption font-medium text-fg-subtle" @click="emit('toggle')">
      <button
        type="button"
        :aria-expanded="open"
        class="flex min-w-0 flex-1 cursor-default items-center text-left"
        @click.stop="emit('toggle')"
      >
        <span class="shrink-0">{{ t('context.cost.title') }}</span>
        <span v-if="open && price !== null" class="ml-1.5 truncate tabular-nums text-fg-emphasis">{{ formatDollars(price) }}</span>
      </button>
      <template #end>
        <span v-if="offered" class="mr-1" @click.stop><Segmented v-model="range" :options="rangeOptions" size="sm" /></span>
        <FoldChevron :open="open" :size="ICON_PX.in20" />
      </template>
    </SectionRow>
    <!--
      Closed, two figures side by side: the price at the text's left edge, the
      rate at the middle.
    -->
    <Fold :open="!open">
      <div class="cost-figures px-3 pb-1">
        <div v-for="figure in figures" :key="figure.key">
          <div class="cost-figure whitespace-nowrap tabular-nums">{{ figure.value }}</div>
          <div class="cost-figure-label text-fg-subtle">{{ figure.label }}</div>
        </div>
      </div>
    </Fold>
    <Fold :open="open">
      <ul v-if="kinds.length" class="cost-columns px-3">
        <li v-for="kind in kinds" :key="kind.key" class="flex min-h-6 items-center gap-2">
          <span class="min-w-0 flex-1 truncate text-fg-body">{{ kind.label }}</span>
          <span class="whitespace-nowrap tabular-nums text-fg-subtle">{{ formatDollars(kind.price) }}</span>
        </li>
      </ul>
      <template v-if="rows.length">
        <div class="cost-caption px-3">
          <span class="text-caption font-medium text-fg-subtle">{{ t('context.cost.profiles') }}</span>
        </div>
        <ul class="cost-columns px-3">
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
      <template v-if="rate !== null || charted">
        <div class="cost-caption px-3">
          <span class="text-caption font-medium">
            <span class="text-fg-subtle">{{ t('context.cost.hitRate') }}</span>
            <span v-if="rate !== null" class="ml-1.5 tabular-nums text-fg-emphasis">{{ percent.format(rate) }}</span>
          </span>
        </div>
        <div v-if="chart && charted" class="px-3">
          <CacheHitChart :chart="chart" :pinned-tip="pinnedTip" />
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

/*
 * Two columns of cells, row by row, each its name with its figure at its end,
 * as the kinds of token and the profiles stand.
 */
.cost-columns {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  column-gap: 1rem;
}

.cost-caption {
  display: flex;
  align-items: center;
  min-height: 1.75rem;
  padding-top: 0.5rem;
  padding-bottom: 0.125rem;
}
</style>
