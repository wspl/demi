<script setup lang="ts" generic="T extends string">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import type { Component } from 'vue'
import { pressOnKey } from './button-events'
import { disabledTooltip } from './disabled'
import { useIconPx } from './icon-metrics'
import { clickedChoice } from './segmented'
import Tooltip from './Tooltip.vue'
import { isTextCut } from './truncation'
import type { SentenceText, TitleText } from './ui-text'

/**
 * One of a few exclusive choices, all visible. Segments share one width so the
 * thumb is a single element that slides to the chosen one instead of re-appearing.
 * That width fits the longest label, and a settings row that drops the
 * control under its label stretches it (`data-field`, except with `iconOnly`
 * whose segments stay square): the segments then share the row's width, each
 * label whole. In a container narrower than that, as a narrow work panel's
 * toolbar, the segments narrow together, as macOS's do, each label cut short
 * with an ellipsis and whole in its tooltip, down to its first few characters
 * (3em); in a container narrower still, as on a phone with many segments, they
 * scroll inside their own box instead of squeezing their labels into each
 * other, and the chosen one scrolls into view. Icon segments never narrow;
 * they only scroll.
 * With `iconOnly`, each segment shows its icon alone and its label as the
 * tooltip and the accessible name, and is square: as wide as it is tall, the
 * icon in its centre.
 * Two options are one toggle: a click anywhere on the control, the chosen
 * segment and the frame around and between the segments included, switches
 * to the other. With three or more, a click chooses the clicked segment.
 */
export interface SegmentedOption<T extends string> {
  value: T
  label: TitleText
  icon?: Component
}

const props = withDefaults(defineProps<{
  options: readonly SegmentedOption<T>[]
  size?: 'sm' | 'md'
  /** Icons without labels; every option then needs an icon. */
  iconOnly?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: SentenceText
}>(), {
  size: 'md',
})

const model = defineModel<T>({ required: true })
const icons = useIconPx()
const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))

const selectedIndex = computed(
  () =>
    Math.max(0, props.options.findIndex((option) => option.value === model.value))
)

const segments = ref<HTMLElement | null>(null)

/** A labelled segment's tooltip shows only while its label is cut. */
function labelCut(trigger: HTMLElement): boolean {
  const label = trigger.querySelector<HTMLElement>('[data-segment-label]')
  return label !== null && isTextCut(label)
}

/** Scrolls the segments' own box, never the page, so that the chosen one shows whole. */
function revealChosen(): void {
  const box = segments.value?.parentElement
  const chosen = segments.value?.querySelectorAll<HTMLElement>('[role="radio"]')[selectedIndex.value]
  if (!box || !chosen) {
    return
  }
  const left = chosen.offsetLeft
  const right = left + chosen.offsetWidth
  if (left < box.scrollLeft) {
    box.scrollLeft = left
  } else if (right > box.scrollLeft + box.clientWidth) {
    box.scrollLeft = right - box.clientWidth
  }
}

onMounted(revealChosen)
watch([selectedIndex, () => props.options], () => nextTick(revealChosen))

/**
 * A radio group's keys: the arrows move the choice and its focus to the
 * neighbouring segment, wrapping round, and Return and Space press the
 * focused one as a click does.
 */
const ARROW_STEP: Record<string, number> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }

function onSegmentKey(event: KeyboardEvent, index: number): void {
  const step = ARROW_STEP[event.key]
  if (props.disabled || step === undefined || event.type !== 'keydown') {
    pressOnKey(event)
    return
  }
  event.preventDefault()
  const next = (index + step + props.options.length) % props.options.length
  model.value = props.options[next]!.value
  void nextTick(() => segments.value?.querySelectorAll<HTMLElement>('[role="radio"]')[next]?.focus())
}

/** `clicked` is the segment's value, or undefined for the frame around and between them. */
function choose(clicked: T | undefined) {
  if (props.disabled)
    return
  const values = props.options.map((option) => option.value)
  model.value = clickedChoice(values, model.value, clicked)
}
</script>

<template>
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="div"
    class="inline-flex max-w-full overflow-x-auto [scrollbar-width:none]"
    :data-field="iconOnly ? undefined : true"
    :open-delay-ms="80"
  >
    <!-- Labelled segments are equal columns that may narrow to nothing, so the thumb's share stays theirs;
         the grid's least width keeps each label's first 3em with its padding, then the box scrolls. -->
    <div
      ref="segments"
      class="relative grid grow grid-flow-col rounded-md bg-(--fill-color) p-[2px] [--fill-color:color-mix(in_srgb,var(--surface-current),var(--overlay)_6%)] *:on-fill"
      :class="[
        iconOnly
          ? 'w-max shrink-0 auto-cols-fr'
          : [
              'auto-cols-[minmax(0,1fr)] min-w-[calc(var(--segment-least)*var(--segments)+4px)]',
              // 3em of the label's size, with the segment's padding; a phone pads its segments wider.
              size === 'sm'
                ? '[--segment-least:calc(3*var(--text-control-xs)+--spacing(3))] phone:[--segment-least:calc(3*var(--text-control-xs)+--spacing(6))]'
                : '[--segment-least:calc(3*var(--text-control-sm)+--spacing(4))] phone:[--segment-least:calc(3*var(--text-control-sm)+--spacing(6))]',
            ],
        disabled ? 'pointer-events-none cursor-not-allowed opacity-40' : '',
      ]"
      :style="{ '--segments': options.length }"
      role="radiogroup"
      :aria-disabled="disabled || undefined"
      @click="choose(undefined)"
    >
      <span
        aria-hidden="true"
        class="segmented-thumb pointer-events-none absolute inset-y-[2px] left-[2px] rounded-[5px] transition-transform duration-200 ease-out motion-reduce:transition-none"
        :style="{
        width: `calc((100% - 4px) / ${options.length})`,
        transform: `translateX(${selectedIndex * 100}%)`,
      }"
      />
      <Tooltip
        v-for="(option, index) in options"
        :key="option.value"
        tag="span"
        :content="option.label"
        :show-if="iconOnly ? undefined : labelCut"
        :open-delay-ms="80"
        role="radio"
        :aria-checked="model === option.value"
        :aria-label="iconOnly ? option.label : undefined"
        :tabindex="disabled ? undefined : index === selectedIndex ? 0 : -1"
        class="relative z-10 inline-flex min-w-0 cursor-default select-none items-center justify-center gap-1 whitespace-nowrap rounded-[5px] transition-colors duration-200 ease-out"
        :class="[
          // A phone draws either size at its segment's height (`base.css`).
          iconOnly
            ? (size === 'sm' ? 'size-5 phone:size-segment' : 'size-segment')
            : (size === 'sm' ? 'px-1.5 py-0.5 text-control-xs leading-4 phone:h-segment phone:px-3 phone:py-0' : 'h-segment px-2 text-control-sm phone:px-3'),
          model === option.value ? 'text-fg-emphasis' : 'text-fg-subtle hover:text-fg',
        ]"
        @click.stop="choose(option.value)"
        @keydown="onSegmentKey($event, index)"
        @keyup="onSegmentKey($event, index)"
      >
        <component
          :is="option.icon"
          v-if="option.icon"
          :size="icons.in24"
          class="shrink-0"
        />
        <span v-if="!iconOnly" data-segment-label class="min-w-0 truncate">{{ option.label }}</span>
      </Tooltip>
    </div>
  </Tooltip>
</template>

<style scoped>
/* The thumb has no edge: its fill alone parts it from the track. In light
   mode it is a button on the track; in dark mode a light button would glare,
   so it takes the selected wash a menu's current row has, over the track. */
.segmented-thumb {
  background: color-mix(in srgb, var(--surface-current), var(--btn-mix));
}

html[data-theme="dark"] .segmented-thumb {
  background: color-mix(in srgb, var(--surface-current), var(--active-mix));
}
</style>
