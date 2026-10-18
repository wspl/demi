<script setup lang="ts">
import { t as shared } from '../locales/text'
import { computed, inject } from 'vue'
import { SquareTerminal, X } from '@lucide/vue'
import type { ConversationStatus } from './conversation-status'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import ConversationStatusDot from './ConversationStatusDot.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { iconTabsKey } from './tab-strip'

/**
 * One tab of a `TabStrip`. The strip's tabs share its room equally: each grows from nothing by the same amount
 * (`basis-0 grow`) until it fits its content, up to 160px, and the room a shorter one leaves goes to the others, so
 * every tab whose title is cut has the same width and a longer title ends in an ellipsis instead of widening its tab.
 * In a crowded strip they narrow, as Chrome's tabs do, down to their mark, a few characters of their title and their
 * close slot (`--tab-min`), and only then does the strip scroll; on a phone a tab keeps its content's width, and the
 * strip scrolls at once. Its width is the same selected or not. A faint line sits in the
 * gap after the tab and fades out while the tab or its neighbour is active or
 * hovered. The close control has a slot of its own at the tab's end, kept on
 * every tab so a tab never changes width when the pointer reaches it. The
 * control shows on the active tab and on the tab under the pointer, and the
 * title is cut short before it, never under it; on any other tab the title
 * runs on over the empty slot, so a narrow tab shows no gap before the next. The root stays a single element with no top-level comment: a
 * fragment root would keep the strip's enter and leave transitions off it.
 *
 * It acts as a web browser's tab does: the primary button selects it as it
 * goes down, so a drag the strip starts there carries the selected tab; a
 * middle click closes it; it takes the keyboard's focus, where Enter and
 * Space select it and the strip's arrows move between tabs (`TabStrip`).
 * A right-click asks for its menu, as do the Menu key and Shift+F10 while it
 * has the focus, with the menu at its lower left. A tab that is not
 * `closable` has no close control and no middle-click close: a click only
 * selects it, and closing is in its menu.
 *
 * In a window whose tabs narrow to their icons (`iconTabsKey`) a phone shows each tab other than the selected one
 * as its mark with its status dot, 44px wide at least, its name left to assistive technology; the selected tab adds
 * its name, cut short. A tab is shorter than its bar (`--spacing-tab`), on a phone too, so the selected one stands
 * inside the strip with room above and below rather than running into the view under it.
 */
const props = withDefaults(defineProps<{
  title: string
  isActive: boolean
  /** The dot on the mark; none without it. */
  status?: ConversationStatus
  /** The built-in mark; the `mark` slot replaces it, as an agent's tab's `AgentMark` does. */
  mark?: 'terminal'
  /** The tab is at work, as a loading page is: a spinner stands in for its mark. */
  busy?: boolean
  /** A picture in place of the mark, such as a web page's icon, by its URL. */
  icon?: string | null
  closable?: boolean
}>(), {
  closable: true,
})

// Whether this tab is an icon on a phone; CSS applies it only in the phone layout (the `phone:` variant).
const iconsOnPhone = inject(iconTabsKey, false)
const iconOnly = computed(() => iconsOnPhone && !props.isActive)

const emit = defineEmits<{
  select: []
  close: []
}>()

function onPointerdown(event: PointerEvent): void {
  if (event.button === 0) {
    emit('select')
  }
}

// A middle click closes the tab once it is released over it, as a browser's tab does.
function onAuxclick(event: MouseEvent): void {
  if (event.button === 1 && props.closable) {
    emit('close')
  }
}

</script>

<template>
  <span
    role="tab"
    :aria-selected="isActive"
    tabindex="0"
    class="group relative flex h-(--tab-h) [--surface-current:var(--fill-color)] [--tab-h:var(--spacing-tab)] min-w-[min(var(--tab-min),var(--tab-room))] max-w-max shrink grow basis-0 phone:shrink-0 phone:grow-0 phone:basis-auto cursor-default items-center rounded-md text-chrome select-none after:absolute after:-right-[1.5px] after:top-1/2 after:h-3.5 after:w-px after:-translate-y-1/2 after:bg-line after:transition-opacity after:duration-150 last:after:hidden hover:after:opacity-0 has-[+:hover]:after:opacity-0 has-[+[aria-selected=true]]:after:opacity-0"
    :class="[iconOnly ? 'phone:w-11 phone:min-w-11' : '', isActive
      ? 'bg-(--tab-active) text-fg-emphasis after:opacity-0 [--fill-color:var(--tab-active)]'
      : 'text-fg-subtle hover:bg-(--tab-hover) hover:text-fg-body hover:[--fill-color:var(--tab-hover)]']"
    @pointerdown="onPointerdown"
    @mousedown.middle.prevent
    @auxclick="onAuxclick"
    @keydown.enter.self="emit('select')"
    @keydown.space.self.prevent="emit('select')"
  >
    <!-- The tooltip covers the whole tab; the tab itself stays a plain element so the strip can transition it.
         Its cap is the tab's: the tab grows to its content (`max-w-max`), which this holds to 160px and the view. -->
    <Tooltip
      tag="span"
      :content="title"
      placement="bottom"
      class="flex h-full min-w-0 max-w-[min(--spacing(40),var(--tab-room))] flex-1 items-center"
      :class="iconOnly ? 'phone:justify-center' : ''"
    >
    <!-- The strip finds the mark by its attribute, to cover one its edge cuts. -->
    <span
      v-if="$slots.mark || mark || icon"
      data-tab-mark
      class="relative ml-1.5 flex shrink-0 items-center justify-center"
      :class="iconOnly ? 'phone:ml-0' : ''"
    >
      <IndeterminateSpinner v-if="busy" :size="ICON_PX.markIn28" class="text-fg-subtle" />
      <img v-else-if="icon" :src="icon" alt="" :width="ICON_PX.markIn28" :height="ICON_PX.markIn28" draggable="false">
      <slot v-else name="mark">
        <SquareTerminal
          :size="ICON_PX.markIn28"
          class="text-fg-subtle"
        />
      </slot>
      <ConversationStatusDot v-if="status" :status="status" />
    </span>
    <!-- While the close control hides, the title runs on over its slot, as a browser's tab's does;
         the slot stays, so the tab keeps its width when the control shows. -->
    <span
      class="min-w-0 flex-1 pl-1.5"
      :class="[closable ? 'pr-1' : 'pr-2', iconOnly ? 'phone:sr-only' : '']"
    >
      <span
        class="block w-max truncate whitespace-nowrap"
        :class="isActive || !closable ? 'max-w-full' : 'max-w-[calc(100%_+_var(--spacing-hit-xs))] group-hover:max-w-full phone:max-w-full'"
      >{{ title }}</span>
    </span>
    <!-- The close control's own slot: the title ends before it, and it sits as far from the
         tab's right edge as centering puts it from the top and bottom. -->
    <span v-if="closable" class="flex shrink-0 items-center mr-[calc((var(--tab-h)_-_var(--spacing-hit-xs))/2)]">
      <IconButton
        :icon="X"
        size="xs"
        variant="ghost"
        :aria-label="shared('common.close')"
        class="transition-opacity"
        :class="isActive ? '' : 'opacity-0 group-hover:opacity-100 phone:opacity-100'"
        @pointerdown.stop
        @click.stop="emit('close')"
      />
    </span>
    </Tooltip>
  </span>
</template>
