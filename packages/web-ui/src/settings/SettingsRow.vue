<script setup lang="ts">
import { computed, ref, useSlots } from 'vue'
import { useElementSize } from '@vueuse/core'
import { ChevronRight } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import { disabledTooltip } from '../ui/disabled'
import { provideRowTarget } from '../ui/operations'
import RowSpinner from '../ui/RowSpinner.vue'
import Tag from '../ui/Tag.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import type { SettingsRowStatus } from './types'
import type { SentenceText } from '../ui/ui-text'

/**
 * Label and explanation on the left, the control on the right. Icons align with
 * the title. Controls center, or align with the title when detail is present.
 * The row decides by its own content whether the two fit side by side: the
 * label keeps at least 9rem (11rem on a phone), and where the control, at its own
 * width, does not leave that, the control drops under the text, as in iOS
 * Settings, and takes the whole line. The line is a flex-wrap line, so what
 * the text and the control need decides it, in every language and in every
 * plugin's page, with no width written in any row. A title wraps at its words
 * and is cut only where one word is wider than the line. On its own line a
 * field (an element marked `data-field`: a text input, a text area, a slider,
 * a button-style dropdown, a segmented control) stretches across the row, a
 * segmented control's segments sharing the width; every other control (a
 * switch, a button) keeps its size and its right alignment.
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
 * keeps its width: where the tags do not fit beside it they move under it
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
  /** The `leading` item is taller than a line of text, such as a 32 px avatar: it centres on the row instead of sitting on the name's line. */
  centerLeading?: boolean
  isolateControls?: boolean
  muted?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: SentenceText
  statuses?: readonly SettingsRowStatus[]
  /**
   * What the row stands for, as operations name it, such as `passkey:k1`:
   * the row shows the spinner while any operation on it runs.
   */
  target?: string
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

// The row works while an operation on its target runs, whichever control
// started it, a dialog since closed included (`operations.ts`): a spinner
// stands before its controls, and its accessory, such as its ··· menu, stays
// for the menu to be opened again.
const working = provideRowTarget(() => props.target)
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
    <!-- Wraps where the controls do not fit beside the text, and always with `below`, whose line is its own. -->
    <div
      class="flex min-w-0 flex-1 items-center"
      :class="[
      inset ? 'min-h-9 phone:min-h-11 gap-x-3 bg-(--fill-color) py-1.5 pl-7 pr-4 [--fill-color:color-mix(in_srgb,var(--surface-current),var(--overlay)_2.5%)] *:on-fill' : compact ? (below ? 'gap-x-3 px-3 py-3' : 'min-h-10 phone:min-h-11 gap-x-3 px-3 py-1.5') : 'min-h-14 gap-x-4 px-4 py-3',
      below ? 'flex-wrap gap-y-3' : inset ? 'flex-nowrap gap-y-2' : 'flex-wrap gap-y-2',
      accessory ? 'pr-0!' : '',
    ]"
    >
      <div
        v-if="$slots.leading"
        class="flex shrink-0 items-center text-fg-muted"
        :class="[centerLeading ? '' : 'h-5', faded ? 'opacity-60' : '']"
        :style="{ alignSelf: !centerLeading && (description || $slots.description || topAligned) ? 'flex-start' : 'center' }"
      >
        <slot name="leading" />
      </div>
      <!-- The text takes every line's room the controls leave, and never less than 9rem (11rem on a phone, whose controls and text are larger): below that the controls drop. -->
      <div
        class="min-w-0 select-none"
        :class="[inset ? 'flex-1' : 'grow-[1000] basis-36 phone:basis-44', faded ? 'opacity-60' : '']"
      >
        <div
          class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 leading-5 phone:leading-[22px]"
          :class="inset ? 'text-setting-small text-fg-body' : 'text-setting text-fg'"
        >
          <TruncatedText class="max-w-full shrink-0" wrap :text="label" />
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
          class="mt-0.5 min-w-0 text-setting-small leading-4 text-fg-subtle phone:leading-5"
        >
          <slot name="description">{{ description }}</slot>
        </div>
        <div v-if="$slots.detail" class="mt-1.5 min-w-0">
          <slot name="detail" />
        </div>
      </div>
    <!-- The controls keep their own width beside the text, which wraps before they drop.
         Dropped, they take the whole line (they are the only item on it, so their grow is all of it)
         and a field stretches across it. An inset row never drops. -->
      <!-- A row with no controls shows its work at its end, before its accessory. -->
      <RowSpinner v-if="working && !$slots.default" class="ml-auto self-center" />
      <div
        v-if="$slots.default"
        @click="isolateControls && $event.stopPropagation()"
        class="flex min-w-0 items-center justify-end gap-2 self-stretch"
        :class="[
        inset ? 'shrink-0' : 'max-w-full grow *:data-field:grow',
        topAligned && !inset ? 'h-5 self-start' : '',
        topAligned && inset ? '@sm:h-5 @sm:self-start' : '',
        disabled ? 'pointer-events-none' : '',
      ]"
      >
        <RowSpinner v-if="working" />
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
