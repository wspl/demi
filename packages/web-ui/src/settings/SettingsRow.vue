<script setup lang="ts">
import { computed, ref, useSlots } from 'vue'
import { useElementSize } from '@vueuse/core'
import { ChevronRight } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import { disabledTooltip } from '../ui/disabled'
import Tag from '../ui/Tag.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
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
 * `disabledReason` on hover. A navigable row opens a page of its own, as a row
 * of System Settings does: the whole row is its button, and a chevron at its
 * end says so. The `accessory` slot holds such a mark (the chevron, a fold's
 * chevron): it sits in a square as tall as the row at the row's end, so its
 * distance to the row's end equals its distance to the row's top and bottom
 * (the edge-control rule). `statuses` are the row's state, each a label
 * right after the name, after the `tags`. The name tells rows apart, so it
 * keeps its width: where the tags do not fit beside it they move under it,
 * and only a name wider than the whole line is cut, whole in its tooltip
 * (the Writing page's rule for long text). The `below` slot holds what needs
 * the row's whole width, such as an account's usage bars: it takes a line of
 * its own under the name and the controls at every width, from the row's
 * start to its end, and the controls then align with the name's line, as
 * they do beside `detail`.
 */
const props = defineProps<{
  label: SentenceText
  description?: SentenceText
  inset?: boolean
  compact?: boolean
  interactive?: boolean
  /** Opens a page of its own: clickable as a whole, with a chevron at its end. */
  navigable?: boolean
  isolateControls?: boolean
  muted?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: SentenceText
  statuses?: readonly SettingsRowStatus[]
}>()

const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))
const faded = computed(() => props.muted === true || props.disabled === true)
const slots = useSlots()
const clickable = computed(() => props.interactive === true || props.navigable === true)
const accessory = computed(() => props.navigable === true || slots['accessory'] !== undefined)
const below = computed(() => slots['below'] !== undefined)
/** The controls align with the name's line rather than the row's middle. */
const topAligned = computed(() => slots['detail'] !== undefined || below.value)
// The accessory's square: as wide as the row is tall. CSS cannot size a
// stretched flex item's width from its height, so the row is measured.
const rowElement = ref<HTMLElement>()
const { height: rowHeight } = useElementSize(rowElement, undefined, { box: 'border-box' })

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
      ref="rowElement"
      :data-setting="label"
      class="flex items-stretch"
      :class="[clickable ? 'cursor-default' : '', navigable ? 'outline-none focus-visible:bg-hover' : '', disabled ? 'cursor-not-allowed' : '']"
      :role="navigable ? 'button' : undefined"
      :tabindex="navigable && !disabled ? 0 : undefined"
      @click="!disabled && clickable && emit('click')"
      @keydown.enter.self="!disabled && navigable && emit('click')"
      @keydown.space.self.prevent="!disabled && navigable && emit('click')"
    >
    <!-- Wraps under @sm, where the controls drop under the text, and always with `below`, whose line is its own. -->
    <div
      class="flex min-w-0 flex-1 items-center gap-y-2"
      :class="[
      inset ? 'min-h-9 gap-x-3 bg-(--fill-color) py-1.5 pl-7 pr-4 [--fill-color:color-mix(in_srgb,var(--surface-current),var(--overlay)_2.5%)] *:on-fill' : compact ? 'min-h-10 gap-x-3 px-3 py-1.5' : 'min-h-14 gap-x-4 px-4 py-3',
      below ? 'flex-wrap' : inset ? 'flex-nowrap' : 'flex-wrap @sm:flex-nowrap',
      accessory ? 'pr-0!' : '',
    ]"
    >
      <div
        v-if="$slots.leading"
        class="flex h-5 shrink-0 items-center text-fg-muted"
        :style="{ alignSelf: description || $slots.description || topAligned ? 'flex-start' : 'center' }"
        :class="faded ? 'opacity-60' : ''"
      >
        <slot name="leading" />
      </div>
      <div
        class="min-w-0 flex-1 select-none"
        :class="faded ? 'opacity-60' : ''"
      >
        <div
          class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 leading-5"
          :class="inset ? 'text-[12px] text-fg-body' : 'text-chrome text-fg'"
        >
          <TruncatedText class="max-w-full shrink-0" :text="label" />
          <div v-if="$slots.tags || statuses?.length" class="flex min-h-5 min-w-0 flex-wrap items-center gap-1">
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
        topAligned ? '@sm:h-5 @sm:self-start' : '',
        disabled ? 'pointer-events-none' : '',
      ]"
      >
        <slot />
      </div>
      <div v-if="below" class="order-last min-w-0 basis-full" :class="faded ? 'opacity-60' : ''">
        <slot name="below" />
      </div>
    </div>
    <!-- A square as tall as the row: its mark is as far from the row's end as from its top and bottom. -->
    <div
      v-if="accessory"
      class="flex shrink-0 items-center justify-center text-fg-subtle"
      :class="faded ? 'opacity-60' : ''"
      :style="{ width: `${rowHeight}px` }"
    >
      <slot name="accessory">
        <ChevronRight :size="ICON_PX.in28" aria-hidden="true" />
      </slot>
    </div>
    </div>
  </Tooltip>
</template>
