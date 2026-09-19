<script setup lang="ts">
import { Bot, SquareTerminal, X } from '@lucide/vue'
import type { ConversationStatus } from './conversation-status'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import ConversationStatusDot from './ConversationStatusDot.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/**
 * One tab of a `TabStrip`. Content-sized up to 160px; a faint line sits in the
 * gap after the tab and fades out while the tab or its neighbour is active or
 * hovered. The root stays a single element with no top-level comment: a
 * fragment root would keep the strip's enter and leave transitions off it.
 */
defineProps<{
  title: string
  isActive: boolean
  /** The dot on the mark; none without it. */
  status?: ConversationStatus
  /** The built-in mark; the `mark` slot replaces it. */
  mark?: 'bot' | 'terminal'
}>()

const emit = defineEmits<{
  pointerdown: [event: PointerEvent]
  contextmenu: [event: MouseEvent]
  close: []
}>()
</script>

<template>
  <span
    role="tab"
    :aria-selected="isActive"
    class="group relative flex h-7 w-max max-w-40 shrink-0 cursor-default items-center rounded-md text-chrome select-none after:absolute after:-right-[1.5px] after:top-1/2 after:h-3.5 after:w-px after:-translate-y-1/2 after:bg-line after:transition-opacity after:duration-150 last:after:hidden hover:after:opacity-0 has-[+:hover]:after:opacity-0 has-[+[aria-selected=true]]:after:opacity-0"
    :class="isActive
      ? 'bg-(--tab-active) text-fg-emphasis after:opacity-0'
      : 'text-fg-subtle hover:bg-(--tab-hover) hover:text-fg-body'"
    @pointerdown="emit('pointerdown', $event)"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <!-- The tooltip covers the whole tab; the tab itself stays a plain element so the strip can transition it. -->
    <Tooltip
      tag="span"
      :content="title"
      placement="bottom"
      class="flex h-full min-w-0 flex-1 items-center"
    >
    <span
      v-if="$slots.mark || mark"
      class="relative ml-1.5 flex shrink-0 items-center justify-center"
    >
      <slot name="mark">
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
    <span class="min-w-0 flex-1 truncate whitespace-nowrap px-1.5">{{ title }}</span>
    <span
      class="pointer-events-none absolute inset-y-0 right-0 z-10 flex w-9 items-center justify-end overflow-hidden rounded-r-md pr-1.5 opacity-0 transition-opacity group-hover:opacity-100"
    >
      <span
        class="absolute inset-y-0 left-0 w-4"
        :class="
          isActive
            ? 'bg-linear-to-r from-transparent to-(--tab-active)'
            : 'bg-linear-to-r from-transparent to-(--tab-row) group-hover:to-(--tab-hover)'
        "
      />
      <span
        class="absolute inset-y-0 right-0 w-6"
        :class="isActive ? 'bg-(--tab-active)' : 'bg-(--tab-row) group-hover:bg-(--tab-hover)'"
      />
      <span
        role="button"
        aria-label="Close"
        class="pointer-events-auto relative z-10 flex size-5 shrink-0 items-center justify-center rounded text-fg-faint transition-colors hover:bg-hover hover:text-fg-body"
        @pointerdown.stop
        @click.stop="emit('close')"
      >
        <X :size="ICON_PX.in20" />
      </span>
    </span>
    </Tooltip>
  </span>
</template>
