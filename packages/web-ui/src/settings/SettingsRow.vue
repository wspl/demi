<script setup lang="ts">
import { computed } from 'vue'
import { disabledTooltip } from '../ui/disabled'
import Tag from '../ui/Tag.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { SettingsRowStatus } from './types'
import type { SentenceText } from '../ui/ui-text'

/**
 * Label and explanation on the left, the control on the right. Icons align with
 * the title. Controls center, or align with the title when detail is present.
 * In a narrow card the control drops under the text. There a field (an element
 * marked `data-field`: a text input, a text area, a slider, a button-style
 * dropdown) stretches across the row; every other control (a switch, a button,
 * a segmented control) keeps its size and its right alignment. The same
 * container width decides both, so a control fills exactly when it has dropped.
 * An inset row belongs to the row above it (an agent's models). A compact row is
 * for lists of like items (models, accounts); an interactive one opens on click
 * without a hover wash. `muted` fades the label side only, so actions stay at
 * full strength. `disabled` mutes the row, blocks its click, and shows
 * `disabledReason` on hover. `statuses` are the row's state, each a label
 * right after the name, after the `tags`: the name truncates before them, so
 * they never cover the name or the line under it at any width.
 */
const props = defineProps<{
  label: SentenceText
  description?: SentenceText
  inset?: boolean
  compact?: boolean
  interactive?: boolean
  isolateControls?: boolean
  muted?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: SentenceText
  statuses?: readonly SettingsRowStatus[]
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
          <div v-if="$slots.tags || statuses?.length" class="flex h-5 shrink-0 items-center gap-1">
            <slot name="tags" />
            <Tooltip
              v-for="status in statuses"
              :key="status.label"
              :content="status.detail"
              :disabled="!status.detail"
            >
              <Tag :tone="status.tone">{{ status.label }}</Tag>
            </Tooltip>
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
         The container spans the row's content box so a bare input can stretch to it.
         Under the text (below @sm) a field takes the width its siblings leave; an inset row never drops. -->
      <div
        v-if="$slots.default"
        @click="isolateControls && $event.stopPropagation()"
        class="flex min-w-0 items-center justify-end gap-2 self-stretch @sm:basis-auto @sm:max-w-[66%]"
        :class="[
        inset ? 'shrink-0' : 'basis-full @max-sm:*:data-field:flex-1',
        $slots.detail ? '@sm:h-5 @sm:self-start' : '',
        disabled ? 'pointer-events-none' : '',
      ]"
      >
        <slot />
      </div>
    </div>
  </Tooltip>
</template>
