<script setup lang="ts">
import { computed, ref, useTemplateRef, watch } from 'vue'
import { autoUpdate, flip, offset, shift, useFloating } from '@floating-ui/vue'
import { onClickOutside, useElementSize } from '@vueuse/core'
import type { CostCall } from '@demicodes/protocol'
import { pageLanguage } from '../i18n/text'
import { useLayerElevation } from '../overlay/layerElevation'
import { showAbove } from '../overlay/stacking'
import { formatClockAgo } from '../ui/dates'
import { formatDollars } from '../ui/numbers'
import { formatTokens } from '../ui/token-count'
import { type ChartAxis, callHitRate } from './context-cost'
import { t } from './locales/text'

/**
 * The main conversation's turn calls, each at its call's cache hit rate,
 * by its number or by its start time (`usage-and-quota.md` § A
 * conversation's cost): faint lines at 0, 50 and 100%, the numbers or times
 * under the plot, a cache miss a red dot, a compaction a grey dashed line
 * between the calls it separates. Resting the pointer on the plot, or a tap
 * on a phone, shows the nearest call's tip beside its point where it fits,
 * otherwise above it, never past the card's edges.
 */
const props = defineProps<{
  calls: readonly CostCall[]
  axis: ChartAxis
  /** The call whose tip shows without the pointer, as a gallery specimen pins it. */
  pinnedTip?: number | null
}>()

/** The plot's height, four rows of the card. */
const PLOT = 96
/** The plot's top line below the controls above it, and the labels below the plot: a caption's gap. */
const GAP = 4
/** The labels' cap height at the caption's size. */
const CAP = 8
/** The labels' baseline, and the chart's height with their descenders. */
const BASELINE = GAP + PLOT + GAP + CAP
const HEIGHT = BASELINE + 3
/** A label this near an end of the plot is anchored to that end. */
const EDGE = 16
/** The room a time label takes with the gap after it, "12:30 PM" at the caption's size. */
const LABEL_ROOM = 72
/** The round steps of the time axis, in minutes. */
const TIME_STEPS = [5, 10, 15, 30, 60, 120, 240, 480, 1440]

const wrap = useTemplateRef<HTMLElement>('wrap')
const { width } = useElementSize(wrap)

const times = computed(() => props.calls.map((call) => new Date(call.startedAt).getTime()))
const span = computed(() => Math.max(1, times.value.at(-1)! - times.value[0]!))

function xOf(index: number): number {
  const count = props.calls.length
  if (count < 2)
    return width.value / 2
  return props.axis === 'time'
    ? ((times.value[index]! - times.value[0]!) / span.value) * width.value
    : (index / (count - 1)) * width.value
}

function yOf(rate: number): number {
  return GAP + (1 - rate) * PLOT
}

const points = computed(() => props.calls.map((call, index) => ({ x: xOf(index), y: yOf(callHitRate(call)) })))

const line = computed(() => points.value.map((point, index) => `${index ? 'L' : 'M'}${point.x.toFixed(1)},${point.y.toFixed(1)}`).join(''))
const area = computed(() => {
  const first = points.value[0]
  const last = points.value.at(-1)
  if (!first || !last)
    return ''
  return `M${first.x.toFixed(1)},${yOf(0)}${line.value.replace('M', 'L')}L${last.x.toFixed(1)},${yOf(0)}Z`
})

/** Each compaction, halfway between the call before it and the first after it. */
const compactions = computed(() =>
  props.calls.flatMap((call, index) => (call.afterCompaction && index > 0 ? [(xOf(index - 1) + xOf(index)) / 2] : [])),
)
const misses = computed(() => props.calls.flatMap((call, index) => (call.miss === null ? [] : [points.value[index]!])))

const clock = new Intl.DateTimeFormat(pageLanguage(), { hour: 'numeric', minute: '2-digit' })

/** The labels under the plot: the first and last call's numbers and round ones between, or round times. */
const labels = computed(() => {
  const count = props.calls.length
  const at = (x: number, text: string) => ({
    x: x < EDGE ? 0 : x > width.value - EDGE ? width.value : x,
    anchor: x < EDGE ? 'start' : x > width.value - EDGE ? 'end' : 'middle',
    text,
  })
  if (count === 0)
    return []
  if (props.axis === 'time') {
    const start = times.value[0]!
    // The shortest round step whose labels each have a label's room.
    const room = Math.max(1, Math.floor(width.value / LABEL_ROOM))
    const minutes = TIME_STEPS.find((step) => span.value / (step * 60_000) <= room) ?? TIME_STEPS.at(-1)!
    const step = minutes * 60_000
    const marks = []
    for (let moment = Math.ceil(start / step) * step; moment <= times.value.at(-1)!; moment += step)
      marks.push(at(((moment - start) / span.value) * width.value, clock.format(moment)))
    return marks
  }
  const step = count > 100 ? 50 : count > 30 ? 10 : 5
  const numbers = [1]
  for (let number = step; number < count - step / 3; number += step)
    numbers.push(number)
  if (count > 1)
    numbers.push(count)
  return numbers.map((number) => at(xOf(number - 1), number === 1 ? '#1' : String(number)))
})

/** The call under the pointer, or the tap; the pinned one while there is none. */
const pointed = ref<number | null>(null)
const shown = computed(() => pointed.value ?? props.pinnedTip ?? null)
watch(() => props.calls.length, () => {
  if (pointed.value !== null && pointed.value >= props.calls.length)
    pointed.value = null
})

function nearest(event: PointerEvent): number | null {
  const box = wrap.value?.getBoundingClientRect()
  if (!box || props.calls.length === 0)
    return null
  const x = event.clientX - box.left
  let best = 0
  for (let index = 1; index < props.calls.length; index++) {
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
/** The tip never outgrows the card, so above the point it fits whole, its lines wrapping. */
const tipWidth = computed(() => (card.value ? Math.min(TIP_WIDTH, Math.max(0, cardWidth.value - 2 * TIP_INSET)) : TIP_WIDTH))
const tip = useTemplateRef<HTMLElement>('tip')
const { floatingStyles } = useFloating(reference, tip, {
  placement: 'right',
  // Within its card, the card is the tip's offset parent; a phone's sheet
  // moves the card, which a fixed tip would not follow.
  strategy: computed(() => (card.value ? 'absolute' : 'fixed')),
  middleware: computed(() => [
    offset(12),
    // Beside the point, so the line under it stays in view; above it when neither side has room.
    flip({ boundary: card.value, crossAxis: false, fallbackPlacements: ['left', 'top'], fallbackStrategy: 'initialPlacement' }),
    shift({ boundary: card.value, padding: TIP_INSET, crossAxis: true }),
  ]),
  whileElementsMounted: autoUpdate,
  transform: false,
})
const elevation = useLayerElevation()
const z = showAbove()

const percent = new Intl.NumberFormat(pageLanguage(), { style: 'percent', maximumFractionDigits: 1 })
const tipCall = computed(() => (shown.value === null ? null : props.calls[shown.value] ?? null))
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
      <circle v-if="points.length" class="cost-chart-first" :cx="points[0]!.x" :cy="points[0]!.y" r="3" />
      <template v-for="miss in misses" :key="`miss@${miss.x}`">
        <line class="cost-chart-miss-tick" :x1="miss.x" :x2="miss.x" :y1="yOf(1)" :y2="miss.y" />
        <circle class="cost-chart-miss" :cx="miss.x" :cy="miss.y" r="3.5" />
      </template>
      <g v-if="shown !== null && points[shown]">
        <line class="stroke-fg-subtle" :x1="points[shown]!.x" :x2="points[shown]!.x" :y1="GAP" :y2="yOf(0)" />
        <circle
          class="cost-chart-cursor"
          :class="tipCall?.miss ? 'fill-on-danger' : 'fill-on-accent'"
          :cx="points[shown]!.x"
          :cy="points[shown]!.y"
          r="3"
        />
      </g>
    </svg>
    <!-- The tip lives in its card's own layer: the product's stays within
         the card, and a gallery specimen's within its specimen. -->
    <Teleport :to="card ?? 'body'">
      <div
        v-if="tipCall && shown !== null"
        ref="tip"
        class="overlay-shell pointer-events-none w-max rounded-md px-2.5 py-1.5 text-[12px] leading-4 text-fg phone:text-[15px] phone:leading-5"
        :style="{ ...floatingStyles, maxWidth: `${tipWidth}px`, '--elevation': elevation, zIndex: z() }"
        role="tooltip"
      >
        <div class="mb-0.5 font-medium text-fg-emphasis tabular-nums">{{ t('context.cost.request', { number: shown + 1 }) }}</div>
        <div class="mb-1 text-fg-subtle tabular-nums">{{ formatClockAgo(tipCall.startedAt, now) }}</div>
        <div class="cost-tip-row">
          <span class="text-fg-subtle">{{ t('context.cost.hitRate') }}</span>
          <span class="font-medium text-fg-emphasis tabular-nums">{{ percent.format(callHitRate(tipCall)) }}</span>
        </div>
        <template v-if="tipCall.miss">
          <div class="mt-1 text-on-danger">{{ t('context.cost.miss', { cause: t(`context.cost.cause.${tipCall.miss}`) }) }}</div>
          <div class="cost-tip-row">
            <span class="text-fg-subtle">{{ t('context.cost.rewritten') }}</span>
            <span class="tabular-nums">{{ formatTokens(tipCall.inputTokens + tipCall.cacheWriteTokens) }}</span>
          </div>
        </template>
        <div v-else-if="shown === 0" class="mt-1 text-fg-subtle">{{ t('context.cost.firstRequest') }}</div>
        <div v-else-if="tipCall.afterCompaction" class="mt-1 text-fg-subtle">{{ t('context.cost.afterCompaction') }}</div>
        <div v-if="tipCall.price !== null" class="cost-tip-row">
          <span class="text-fg-subtle">{{ t('context.cost.price') }}</span>
          <span class="tabular-nums">{{ formatDollars(tipCall.price) }}</span>
        </div>
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
