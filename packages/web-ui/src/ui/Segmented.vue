<script setup lang="ts" generic="T extends string">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import type { Component } from 'vue'
import { disabledTooltip } from './disabled'
import { ICON_PX } from './icon-metrics'
import { clickedChoice } from './segmented'
import Tooltip from './Tooltip.vue'
import type { SentenceText, TitleText } from './ui-text'

/**
 * One of a few exclusive choices, all visible. Segments share one width so the
 * thumb is a single element that slides to the chosen one instead of re-appearing.
 * That width fits the longest label: in a container narrower than the
 * segments, as on a phone, they scroll inside their own box instead of
 * squeezing their labels into each other, and the chosen one scrolls into
 * view.
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
const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))

const selectedIndex = computed(
  () =>
    Math.max(0, props.options.findIndex((option) => option.value === model.value))
)

const segments = ref<HTMLElement | null>(null)

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
    :open-delay-ms="80"
  >
    <div
      ref="segments"
      class="relative grid w-max shrink-0 auto-cols-fr grid-flow-col rounded-md bg-(--fill-color) p-[2px] [--fill-color:color-mix(in_srgb,var(--surface-current),var(--overlay)_6%)] *:on-fill"
      :class="disabled ? 'pointer-events-none cursor-not-allowed opacity-40' : ''"
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
        v-for="option in options"
        :key="option.value"
        tag="span"
        :content="option.label"
        :disabled="!iconOnly"
        :open-delay-ms="80"
        role="radio"
        :aria-checked="model === option.value"
        :aria-label="iconOnly ? option.label : undefined"
        class="relative z-10 inline-flex cursor-default select-none items-center justify-center gap-1 whitespace-nowrap rounded-[5px] transition-colors duration-200 ease-out"
        :class="[
          iconOnly
            ? (size === 'sm' ? 'size-5' : 'size-6')
            : (size === 'sm' ? 'px-1.5 py-0.5 text-[11px] leading-4' : 'h-6 px-2 text-[12px]'),
          model === option.value ? 'text-fg-emphasis' : 'text-fg-subtle hover:text-fg',
        ]"
        @click.stop="choose(option.value)"
      >
        <component
          :is="option.icon"
          v-if="option.icon"
          :size="ICON_PX.in24"
        />
        <template v-if="!iconOnly">{{ option.label }}</template>
      </Tooltip>
    </div>
  </Tooltip>
</template>

<style scoped>
/* The thumb takes the button's fill from the theme's tokens, in both modes,
   and no edge: its fill alone parts it from the track. */
.segmented-thumb {
  background: color-mix(in srgb, var(--surface-current), var(--btn-mix));
}
</style>
