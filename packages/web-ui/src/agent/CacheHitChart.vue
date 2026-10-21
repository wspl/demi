<script setup lang="ts">
import { computed, ref, useTemplateRef, watch } from 'vue'
import { autoUpdate, flip, offset, shift, useFloating } from '@floating-ui/vue'
import { onClickOutside, useElementSize } from '@vueuse/core'
import { niceTicks } from '@demicodes/utils'
import { pageLanguage } from '../i18n/text'
import { useLayerElevation } from '../overlay/layerElevation'
import { showAbove } from '../overlay/stacking'
import { formatClockAgo, formatClockRange } from '../ui/dates'
import { formatDollars } from '../ui/numbers'
import { formatTokens } from '../ui/token-count'
import type { CacheChart } from '@demicodes/protocol'
import { t } from './locales/text'

/**
 * An agent's recent turn calls, each point a group of consecutive calls at
 * their cache hit rate, by their numbers, as the backend counts them
 * (`packages/web-ui/src/agent/ContextUsageIndicator.vue`).
 *
 * The chart draws:
 *
 * - faint lines at 0, 50 and 100%
 * - round call numbers under the plot
 * - a red dot for a group with a cache miss
 * - a grey dashed line for a compaction, before the group of the first call
 *   after it
 *
 * Resting the pointer on the plot, or a tap on a phone, shows the nearest
 * point's tip beside it where it fits, otherwise above it, never past the
 * card's edges: a call's own, or a group's range, rate, price and misses.
 */
const props = defineProps<{
  /** The chart, at least one point of it. */
  chart: CacheChart
  /**
   * The call, by its place among all of the conversation's, whose tip shows
   * without the pointer, as a gallery specimen pins it.
   */
  pinnedTip?: number | null
}>()

/** The plot's height. */
const PLOT = 32
/**
 * The gap of one caption: between the plot's top line and the controls above
 * it, and between the plot and the labels below it.
 */
const GAP = 4
/** The labels' cap height at the caption's size. */
const CAP = 8
/** The labels' baseline, and the chart's height with their descenders. */
const BASELINE = GAP + PLOT + GAP + CAP
const HEIGHT = BASELINE + 3
/** A label this near an end of the plot is anchored to that end. */
const EDGE = 16
/** The plot's width for each label under it, about. */
const LABEL_EVERY = 60

const wrap = useTemplateRef<HTMLElement>('wrap')
const { width } = useElementSize(wrap)

const groups = computed(() => props.chart.points)

function xOf(index: number): number {
  const count = groups.value.length
  if (count < 2)
    return width.value / 2
  return (index / (count - 1)) * width.value
}

function yOf(rate: number): number {
  return GAP + (1 - rate) * PLOT
}

const points = computed(() => groups.value.map((group, index) => ({ x: xOf(index), y: yOf(group.rate) })))

const line = computed(() => points.value.map((point, index) => `${index ? 'L' : 'M'}${point.x.toFixed(1)},${point.y.toFixed(1)}`).join(''))
const area = computed(() => {
  const first = points.value[0]
  const last = points.value.at(-1)
  if (!first || !last)
    return ''
  return `M${first.x.toFixed(1)},${yOf(0)}${line.value.replace('M', 'L')}L${last.x.toFixed(1)},${yOf(0)}Z`
})

/**
 * Each compaction, halfway between the group before it and the group of the
 * first call after it.
 */
const compactions = computed(() =>
  groups.value.flatMap((group, index) => (index > 0 && group.afterCompaction ? [(xOf(index - 1) + xOf(index)) / 2] : [])),
)
const misses = computed(() => groups.value.flatMap((group, index) => (group.misses.length > 0 ? [points.value[index]!] : [])))

/**
 * The labels under the plot: round call numbers, 1, 2 or 5 times a power of
 * ten, about one for each `LABEL_EVERY` pixels. A point stands at the middle
 * of its group's numbers, and a number at its place between the points.
 */
const labels = computed(() => {
  const shown = groups.value
  if (shown.length === 0)
    return []
  const middle = (index: number) => shown[index]!.first + (props.chart.size - 1) / 2
  const from = middle(0)
  const to = shown.length === 1 ? from : middle(shown.length - 1)
  const count = Math.max(1, Math.round(width.value / LABEL_EVERY))
  return niceTicks(from, to, count, 1).map((number) => {
    const x = to === from ? width.value / 2 : ((number - from) / (to - from)) * width.value
    const anchor = x < EDGE ? 'start' : x > width.value - EDGE ? 'end' : 'middle'
    return { x: anchor === 'start' ? 0 : anchor === 'end' ? width.value : x, anchor, text: String(number) }
  })
})

/**
 * The drawn group under the pointer, or the tap; the pinned call's while there
 * is none.
 */
const pointed = ref<number | null>(null)
const pinned = computed(() => {
  const first = groups.value[0]?.first
  if (props.pinnedTip == null || first === undefined)
    return null
  const index = Math.floor((props.pinnedTip + 1 - first) / props.chart.size)
  return index >= 0 && index < groups.value.length ? index : null
})
const shown = computed(() => pointed.value ?? pinned.value)
// A group the drawn ones no longer hold, or one whose place moved as the
// window moved on, has no tip.
watch([() => groups.value.length, () => groups.value[0]?.first], ([length, first], [, earlier]) => {
  if (pointed.value !== null && (pointed.value >= length || first !== earlier))
    pointed.value = null
})

function nearest(event: PointerEvent): number | null {
  const box = wrap.value?.getBoundingClientRect()
  if (!box || groups.value.length === 0)
    return null
  const x = event.clientX - box.left
  let best = 0
  for (let index = 1; index < groups.value.length; index++) {
    if (Math.abs(xOf(index) - x) < Math.abs(xOf(best) - x))
      best = index
  }
  return best
}

function point(event: PointerEvent): void {
  pointed.value = nearest(event)
}

/** A touch's tip stays once the finger lifts, until a tap elsewhere. */
function leave(event: PointerEvent): void {
  if (event.pointerType !== 'touch')
    pointed.value = null
}
onClickOutside(wrap, () => {
  pointed.value = null
})

/** The tip stands at the point, kept inside the card. */
const card = computed(() => wrap.value?.closest<HTMLElement>('[data-context-card]') ?? undefined)
const reference = computed(() => {
  const index = shown.value
  const box = wrap.value
  if (index === null || !box)
    return null
  const { x, y } = points.value[index]!
  return {
    // The chart's element, whose scrolling ancestors move the tip with it.
    contextElement: box,
    getBoundingClientRect: () => {
      const origin = box.getBoundingClientRect()
      return new DOMRect(origin.left + x, origin.top + y, 0, 0)
    },
  }
})
/** The tip's room from the card's edges. */
const TIP_INSET = 4
/** The tip's widest, the width of a tooltip's text. */
const TIP_WIDTH = 320
const { width: cardWidth } = useElementSize(card, undefined, { box: 'border-box' })
/**
 * The tip never outgrows the card, so above the point it fits whole, its lines
 * wrapping.
 */
const tipWidth = computed(() => (card.value ? Math.min(TIP_WIDTH, Math.max(0, cardWidth.value - 2 * TIP_INSET)) : TIP_WIDTH))
const tip = useTemplateRef<HTMLElement>('tip')
const { floatingStyles } = useFloating(reference, tip, {
  placement: 'right',
  // Within its card, the card is the tip's offset parent. A phone's sheet moves
  // the card, which a fixed tip would not follow.
  strategy: computed(() => (card.value ? 'absolute' : 'fixed')),
  middleware: computed(() => [
    offset(12),
    // Beside the point, so the line under it stays in view; above it when
    // neither side has room.
    flip({ boundary: card.value, crossAxis: false, fallbackPlacements: ['left', 'top'], fallbackStrategy: 'initialPlacement' }),
    shift({ boundary: card.value, padding: TIP_INSET, crossAxis: true }),
  ]),
  whileElementsMounted: autoUpdate,
  transform: false,
})
const elevation = useLayerElevation()
const z = showAbove()

const percent = new Intl.NumberFormat(pageLanguage(), { style: 'percent', maximumFractionDigits: 1 })
const tipGroup = computed(() => (shown.value === null ? null : groups.value[shown.value] ?? null))
/** Whether the tip's group is one call, whose own tip it shows. */
const tipCall = computed(() => tipGroup.value !== null && tipGroup.value.first === tipGroup.value.last)
const now = ref(Date.now())
watch(shown, () => {
  now.value = Date.now()
})
</script>

<template>
  <div
    ref="wrap"
    class="relative touch-none select-none"
    :style="{ height: `${HEIGHT}px` }"
    @pointermove="point"
    @pointerdown="point"
    @pointerleave="leave"
  >
    <svg v-if="width > 0" class="block overflow-visible" :width="width" :height="HEIGHT" aria-hidden="true">
      <line v-for="rate in [0, 0.5, 1]" :key="rate" class="stroke-line-subtle" x1="0" :x2="width" :y1="yOf(rate)" :y2="yOf(rate)" />
      <text
        v-for="label in labels"
        :key="`${label.text}@${label.x}`"
        class="cost-chart-label"
        :x="label.x"
        :y="BASELINE"
        :text-anchor="label.anchor"
      >{{ label.text }}</text>
      <path class="cost-chart-area" :d="area" />
      <path class="cost-chart-line" :d="line" />
      <line
        v-for="x in compactions"
        :key="`compaction@${x}`"
        class="cost-chart-compaction"
        :x1="x"
        :x2="x"
        :y1="GAP"
        :y2="yOf(0)"
      />
      <circle v-if="points.length && groups[0]!.first === 1" class="cost-chart-first" :cx="points[0]!.x" :cy="points[0]!.y" r="3" />
      <template v-for="miss in misses" :key="`miss@${miss.x}`">
        <line class="cost-chart-miss-tick" :x1="miss.x" :x2="miss.x" :y1="yOf(1)" :y2="miss.y" />
        <circle class="cost-chart-miss" :cx="miss.x" :cy="miss.y" r="3.5" />
      </template>
      <g v-if="shown !== null && points[shown]">
        <line class="stroke-fg-subtle" :x1="points[shown]!.x" :x2="points[shown]!.x" :y1="GAP" :y2="yOf(0)" />
        <circle
          class="cost-chart-cursor"
          :class="tipGroup?.misses.length ? 'fill-on-danger' : 'fill-on-accent'"
          :cx="points[shown]!.x"
          :cy="points[shown]!.y"
          r="3"
        />
      </g>
    </svg>
    <!--
      The tip lives in its card's own layer. The product's stays within the
      card, and a gallery specimen's within its specimen.
    -->
    <Teleport :to="card ?? 'body'">
      <div
        v-if="tipGroup"
        ref="tip"
        class="overlay-shell pointer-events-none w-max rounded-md px-2.5 py-1.5 text-[12px] leading-4 text-fg phone:text-[15px] phone:leading-5"
        :style="{ ...floatingStyles, maxWidth: `${tipWidth}px`, '--elevation': elevation, zIndex: z() }"
        role="tooltip"
      >
        <template v-if="tipCall">
          <div class="mb-0.5 font-medium text-fg-emphasis tabular-nums">{{ t('context.cost.request', { number: tipGroup.first }) }}</div>
          <div class="mb-1 text-fg-subtle tabular-nums">{{ formatClockAgo(tipGroup.from, now) }}</div>
          <div class="cost-tip-row">
            <span class="text-fg-subtle">{{ t('context.cost.hitRate') }}</span>
            <span class="font-medium text-fg-emphasis tabular-nums">{{ percent.format(tipGroup.rate) }}</span>
          </div>
          <template v-if="tipGroup.misses.length">
            <div class="mt-1 text-on-danger">{{ t('context.cost.miss', { cause: t(`context.cost.cause.${tipGroup.misses[0]}`) }) }}</div>
            <div v-if="tipGroup.rewritten !== null" class="cost-tip-row">
              <span class="text-fg-subtle">{{ t('context.cost.rewritten') }}</span>
              <span class="tabular-nums">{{ formatTokens(tipGroup.rewritten) }}</span>
            </div>
          </template>
          <div v-else-if="tipGroup.first === 1" class="mt-1 text-fg-subtle">{{ t('context.cost.firstRequest') }}</div>
          <div v-else-if="tipGroup.afterCompaction" class="mt-1 text-fg-subtle">{{ t('context.cost.afterCompaction') }}</div>
          <div v-if="tipGroup.price !== null" class="cost-tip-row">
            <span class="text-fg-subtle">{{ t('context.cost.price') }}</span>
            <span class="tabular-nums">{{ formatDollars(tipGroup.price) }}</span>
          </div>
        </template>
        <template v-else>
          <div class="mb-0.5 font-medium text-fg-emphasis tabular-nums">#{{ tipGroup.first }}–#{{ tipGroup.last }}</div>
          <div class="mb-1 text-fg-subtle tabular-nums">
            {{ formatClockRange(tipGroup.from, tipGroup.to, now) }}
          </div>
          <div class="cost-tip-row">
            <span class="text-fg-subtle">{{ t('context.cost.hitRate') }}</span>
            <span class="font-medium text-fg-emphasis tabular-nums">{{ percent.format(tipGroup.rate) }}</span>
          </div>
          <div v-for="cause in tipGroup.misses" :key="cause" class="mt-1 text-on-danger">{{ t('context.cost.miss', { cause: t(`context.cost.cause.${cause}`) }) }}</div>
          <div v-if="tipGroup.price !== null" class="cost-tip-row">
            <span class="text-fg-subtle">{{ t('context.cost.price') }}</span>
            <span class="tabular-nums">{{ formatDollars(tipGroup.price) }}</span>
          </div>
        </template>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
/* The labels in the card's caption style. */
.cost-chart-label {
  font-size: var(--text-caption);
  font-weight: 500;
  fill: var(--color-fg-subtle);
}

.cost-chart-line {
  fill: none;
  stroke: var(--color-on-accent);
  stroke-width: 1.25;
  stroke-linejoin: round;
}

.cost-chart-area {
  fill: color-mix(in srgb, transparent, var(--color-on-accent) 7%);
}

.cost-chart-compaction {
  stroke: var(--color-fg-faint);
  stroke-width: 1;
  stroke-dasharray: 2 2;
}

.cost-chart-miss-tick {
  stroke: var(--color-on-danger);
  stroke-width: 1;
  opacity: 0.45;
}

.cost-chart-miss,
.cost-chart-first,
.cost-chart-cursor {
  stroke: var(--surface-current);
  stroke-width: 1.5;
}

.cost-chart-miss {
  fill: var(--color-on-danger);
}

.cost-chart-first {
  fill: var(--color-fg-faint);
}

/* A tip's line: its name and its figure at the end. */
.cost-tip-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  column-gap: 1rem;
  min-height: 1rem;
}

/* A figure stays on one line; its name wraps when the tip is narrow. */
.cost-tip-row > :last-child {
  white-space: nowrap;
}
</style>
