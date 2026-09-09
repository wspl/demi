<script setup lang="ts">
import { computed } from 'vue'
import { disabledTooltip } from '../ui/disabled'
import Tooltip from '../ui/Tooltip.vue'

/**
 * Label and explanation on the left, the control on the right. Icons align with
 * the title. Controls center by default, or align with the title when detail is
 * present; controlsAlign overrides that choice for the content's visual density.
 * In a narrow card the control drops under the text and keeps its right alignment.
 * An inset row belongs to the row above it (an agent's models). A compact row is
 * for lists of like items (models, accounts); an interactive one opens on click
 * without a hover wash. `muted` fades the label side only, so actions stay at
 * full strength. `disabled` mutes the row, blocks its click, and shows
 * `disabledReason` on hover.
 */
const props = defineProps<{
  label: string
  description?: string
  inset?: boolean
  compact?: boolean
  interactive?: boolean
  isolateControls?: boolean
  muted?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
  controlsAlign?: 'start' | 'center'
}>()

const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))
const faded = computed(() => props.muted === true || props.disabled === true)

const emit = defineEmits<{
  click: []
}>()
</script>

<template>
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="div"
    :open-delay-ms="80"
  >
    <div
      class="flex items-center gap-y-2 @sm:flex-nowrap"
      :class="[
      inset ? 'min-h-9 flex-nowrap gap-x-3 bg-overlay/[0.025] py-1.5 pl-7 pr-4' : compact ? 'min-h-10 flex-wrap gap-x-3 px-3 py-1.5' : 'min-h-14 flex-wrap gap-x-4 px-4 py-3',
      interactive ? 'cursor-default' : '',
      disabled ? 'cursor-not-allowed' : '',
    ]"
      @click="!disabled && interactive && emit('click')"
    >
      <div
        v-if="$slots.leading"
        class="flex h-5 shrink-0 items-center text-fg-muted"
        :style="{ alignSelf: description || $slots.description || $slots.detail ? 'flex-start' : 'center' }"
        :class="faded ? 'opacity-60' : ''"
      >
        <slot name="leading" />
      </div>
      <div
        class="min-w-0 flex-1 select-none"
        :class="faded ? 'opacity-60' : ''"
      >
        <div
          class="flex min-w-0 flex-nowrap items-center gap-x-2 leading-5"
          :class="inset ? 'text-[12px] text-fg-body' : 'text-chrome text-fg'"
        >
          <span class="min-w-0 truncate">{{ label }}</span>
          <div v-if="$slots.tags" class="flex h-5 shrink-0 items-center gap-1">
            <slot name="tags" />
          </div>
        </div>
        <div
          v-if="description || $slots.description"
          class="mt-0.5 min-w-0 text-[12px] leading-4 text-fg-subtle"
        >
          <slot name="description">{{ description }}</slot>
        </div>
        <div v-if="$slots.detail" class="mt-1.5 min-w-0">
          <slot name="detail" />
        </div>
      </div>
    <!-- Beside the text the controls may take up to two thirds; inputs shrink, buttons never wrap.
         The container spans the row's content box so a bare input can stretch to it. -->
      <div
        v-if="$slots.default"
        @click="isolateControls && $event.stopPropagation()"
        class="flex min-w-0 items-center justify-end gap-2 self-stretch @sm:basis-auto @sm:max-w-[66%]"
        :class="[
        inset ? 'shrink-0' : 'basis-full',
        (controlsAlign ?? ($slots.detail ? 'start' : 'center')) === 'start' ? '@sm:h-5 @sm:self-start' : '',
        disabled ? 'pointer-events-none' : '',
      ]"
      >
        <slot />
      </div>
    </div>
  </Tooltip>
</template>
