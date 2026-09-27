<script setup lang="ts" generic="T extends string">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import type { Component } from 'vue'
import { disabledTooltip } from './disabled'
import { ICON_PX } from './icon-metrics'
import Tooltip from './Tooltip.vue'

/**
 * One of a few exclusive choices, all visible. Segments share one width so the
 * thumb is a single element that slides to the chosen one instead of re-appearing.
 * That width fits the longest label: in a container narrower than the
 * segments, as on a phone, they scroll inside their own box instead of
 * squeezing their labels into each other, and the chosen one scrolls into
 * view.
 * With `iconOnly`, each segment shows its icon alone and its label as the
 * tooltip and the accessible name.
 */
export interface SegmentedOption<T extends string> {
  value: T
  label: string
  icon?: Component
}

const props = withDefaults(defineProps<{
  options: readonly SegmentedOption<T>[]
  size?: 'sm' | 'md'
  /** Icons without labels; every option then needs an icon. */
  iconOnly?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
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

function select(value: T) {
  if (props.disabled)
    return
  model.value = value
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
      class="relative grid w-max shrink-0 auto-cols-fr grid-flow-col rounded-md bg-overlay/6 p-[2px]"
      :class="disabled ? 'pointer-events-none cursor-not-allowed opacity-40' : ''"
      role="radiogroup"
      :aria-disabled="disabled || undefined"
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
            ? (size === 'sm' ? 'h-5 px-1.5' : 'h-6 px-2')
            : (size === 'sm' ? 'px-1.5 py-0.5 text-[11px] leading-4' : 'h-6 px-2 text-[12px]'),
          model === option.value ? 'text-fg-emphasis' : 'text-fg-subtle hover:text-fg',
        ]"
        @click="select(option.value)"
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
.segmented-thumb {
  background: var(--btn-bg);
  box-shadow: var(--shadow-btn);
}

html[data-theme="dark"] .segmented-thumb {
  /* Blend with the actual parent surface so the thumb stays lighter on panels. */
  background: color-mix(in srgb, var(--color-overlay) 20%, transparent);
}
</style>
