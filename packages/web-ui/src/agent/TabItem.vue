<script setup lang="ts">
import { Bot, SquareTerminal, X } from '@lucide/vue'
import type { ConversationStatus } from './conversation-status'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import ConversationStatusDot from './ConversationStatusDot.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/**
 * One tab of a `TabStrip`. Content-sized up to 160px, or the strip's room when it has less; in a crowded strip it
 * shrinks, as Chrome's tabs do, down to its mark, a few characters of its title and its close slot (88px), and only
 * then does the strip scroll. A faint line sits in the
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
 */
defineProps<{
  title: string
  isActive: boolean
  /** The dot on the mark; none without it. */
  status?: ConversationStatus
  /** The built-in mark; the `mark` slot replaces it. */
  mark?: 'bot' | 'terminal'
  /** The tab is at work, as a loading page is: a spinner stands in for its mark. */
  busy?: boolean
  /** A picture in place of the mark, such as a web page's icon, by its URL. */
  icon?: string | null
}>()

const emit = defineEmits<{
  select: []
  contextmenu: [event: MouseEvent]
  close: []
}>()

function onPointerdown(event: PointerEvent): void {
  if (event.button === 0) {
    emit('select')
  }
}

// A middle click closes the tab once it is released over it, as a browser's tab does.
function onAuxclick(event: MouseEvent): void {
  if (event.button === 1) {
    emit('close')
  }
}
</script>

<template>
  <span
    role="tab"
    :aria-selected="isActive"
    tabindex="0"
    class="group relative flex h-(--tab-h) w-max [--surface-current:var(--fill-color)] [--tab-h:--spacing(7)] min-w-[min(--spacing(22),var(--tab-room,--spacing(22)))] max-w-[min(--spacing(40),var(--tab-room,--spacing(40)))] shrink cursor-default items-center rounded-md text-chrome select-none after:absolute after:-right-[1.5px] after:top-1/2 after:h-3.5 after:w-px after:-translate-y-1/2 after:bg-line after:transition-opacity after:duration-150 last:after:hidden hover:after:opacity-0 has-[+:hover]:after:opacity-0 has-[+[aria-selected=true]]:after:opacity-0"
    :class="isActive
      ? 'bg-(--tab-active) text-fg-emphasis after:opacity-0 [--fill-color:var(--tab-active)]'
      : 'text-fg-subtle hover:bg-(--tab-hover) hover:text-fg-body hover:[--fill-color:var(--tab-hover)]'"
    @pointerdown="onPointerdown"
    @mousedown.middle.prevent
    @auxclick="onAuxclick"
    @keydown.enter.self="emit('select')"
    @keydown.space.self.prevent="emit('select')"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <!-- The tooltip covers the whole tab; the tab itself stays a plain element so the strip can transition it. -->
    <Tooltip
      tag="span"
      :content="title"
      placement="bottom"
      class="flex h-full min-w-0 flex-1 items-center"
    >
    <!-- The strip finds the mark by its attribute, to cover one its edge cuts. -->
    <span
      v-if="$slots.mark || mark || icon"
      data-tab-mark
      class="relative ml-1.5 flex shrink-0 items-center justify-center"
    >
      <IndeterminateSpinner v-if="busy" :size="ICON_PX.markIn28" class="text-fg-subtle" />
      <img v-else-if="icon" :src="icon" alt="" :width="ICON_PX.markIn28" :height="ICON_PX.markIn28" draggable="false">
      <slot v-else name="mark">
        <Bot
          v-if="mark === 'bot'"
          :size="ICON_PX.markIn28"
          class="text-fg-subtle"
        />
        <SquareTerminal
          v-else
          :size="ICON_PX.markIn28"
          class="text-fg-subtle"
        />
      </slot>
      <ConversationStatusDot v-if="status" :status="status" />
    </span>
    <!-- While the close control hides, the title runs on over its slot, as a browser's tab's does;
         the slot stays, so the tab keeps its width when the control shows. -->
    <span class="min-w-0 flex-1 pl-1.5 pr-1">
      <span
        class="block w-max truncate whitespace-nowrap"
        :class="isActive ? 'max-w-full' : 'max-w-[calc(100%_+_var(--spacing-hit-xs))] group-hover:max-w-full'"
      >{{ title }}</span>
    </span>
    <!-- The close control's own slot: the title ends before it, and it sits as far from the
         tab's right edge as centering puts it from the top and bottom. -->
    <span class="flex shrink-0 items-center mr-[calc((var(--tab-h)_-_var(--spacing-hit-xs))/2)]">
      <IconButton
        :icon="X"
        size="xs"
        variant="ghost"
        aria-label="Close"
        class="transition-opacity"
        :class="isActive ? '' : 'opacity-0 group-hover:opacity-100'"
        @pointerdown.stop
        @click.stop="emit('close')"
      />
    </span>
    </Tooltip>
  </span>
</template>
