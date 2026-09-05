<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { Archive, Pin, PinOff } from '@lucide/vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SidebarConversation } from './types'

/** Wait before scrolling a hovered title; the row layout has settled by then. */
const MARQUEE_DELAY_MS = 500
/** Constant scroll speed and spacing between repeated titles. */
const MARQUEE_PX_PER_S = 36
const MARQUEE_GAP_PX = 24

const props = defineProps<{
  conversation: SidebarConversation
  /** The conversation the session shows. */
  open: boolean
  /** Part of the current selection. */
  selected: boolean
  /** The keyboard cursor is here. */
  focused: boolean
  /** The row's menu is showing, so it stays lit and its actions stay out. */
  menuOpen: boolean
  renaming: boolean
  hidePin?: boolean
}>()

const emit = defineEmits<{
  click: [event: MouseEvent]
  contextmenu: [event: MouseEvent]
  archive: []
  renameSubmit: [title: string]
  renameCancel: []
  togglePin: []
}>()

const renameInputRef = ref<HTMLInputElement>()
const renameValue = ref(props.conversation.title)

watch(
  () => props.renaming,
  (renaming) => {
    if (!renaming) return
    renameValue.value = props.conversation.title
    nextTick(() => {
      renameInputRef.value?.focus()
      renameInputRef.value?.select()
    })
  },
)

// One quiet mark: a breathing dot while running, green for a result waiting to be read, orange
// when the conversation needs the user (it failed or was stopped). Nothing when settled.
const dotClass = computed(() => {
  const { status, unread } = props.conversation
  if (status === 'active') return 'sidebar-breath bg-fg'
  if (status === 'error' || status === 'aborted') return 'bg-on-warning'
  if (status === 'done' && unread) return 'bg-on-success'
  return null
})

// Selected rows are lit; the open one is also emphasized, so it stays visible inside a wider selection.
const rowClass = computed(() => [
  props.selected
    ? props.open
      ? 'bg-active text-fg-emphasis'
      : 'bg-active text-fg'
    : props.menuOpen
      ? 'bg-hover text-fg'
      : 'text-fg-body hover:bg-hover hover:text-fg',
  props.focused ? 'ring-1 ring-inset ring-line-focus' : '',
])

// A long title plays as a marquee while hovered instead of staying cut.
const titleClip = ref<HTMLElement>()
const titleText = ref<HTMLElement>()
const marquee = ref<{ ms: number } | null>(null)
let hoverTimer: ReturnType<typeof setTimeout> | undefined

function startMarquee(): void {
  clearTimeout(hoverTimer)
  hoverTimer = setTimeout(() => {
    const clip = titleClip.value
    const text = titleText.value
    if (!clip || !text) return
    const width = text.getBoundingClientRect().width
    if (width - clip.clientWidth <= 2) return
    marquee.value = { ms: ((width + MARQUEE_GAP_PX) / MARQUEE_PX_PER_S) * 1000 }
  }, MARQUEE_DELAY_MS)
}

function stopMarquee(): void {
  clearTimeout(hoverTimer)
  marquee.value = null
}

onBeforeUnmount(() => clearTimeout(hoverTimer))
</script>

<template>
  <div
    class="group/row relative flex h-7 cursor-default select-none items-center gap-2 rounded-md pl-2 pr-1 text-chrome transition-colors duration-200 ease-out"
    :class="rowClass"
    :aria-selected="selected"
    @click="emit('click', $event)"
    @contextmenu.prevent="emit('contextmenu', $event)"
    @mouseenter="startMarquee"
    @mouseleave="stopMarquee"
  >
    <span class="flex size-3.5 shrink-0 items-center justify-center">
      <span v-if="dotClass" class="size-1.5 rounded-full" :class="dotClass" />
    </span>
    <input
      v-if="renaming"
      ref="renameInputRef"
      v-model="renameValue"
      class="min-w-0 flex-1 bg-transparent font-normal outline-none"
      @keydown.enter.stop="emit('renameSubmit', renameValue)"
      @keydown.escape.stop="emit('renameCancel')"
      @keydown.stop
      @blur="emit('renameSubmit', renameValue)"
      @click.stop
    />
    <!-- The title has the row until hover; then it yields the end to the actions and, if cut, plays.
         A cut title fades out at the edge instead of ending in an ellipsis. -->
    <span
      v-else
      ref="titleClip"
      class="sidebar-title min-w-0 flex-1 overflow-hidden whitespace-nowrap transition-[margin] duration-[80ms] ease-out"
      :class="[
        marquee ? 'is-playing' : '',
        conversation.unread && !open ? 'text-fg-emphasis' : '',
        menuOpen
          ? 'mr-[50px]'
          : conversation.pinned
            ? 'mr-8 group-hover/row:mr-[50px]'
            : 'group-hover/row:mr-[50px]',
      ]"
    >
      <span
        v-if="marquee"
        class="sidebar-marquee inline-flex w-max"
        :style="{ '--marquee-gap': `${MARQUEE_GAP_PX}px`, '--marquee-ms': `${marquee.ms}ms` }"
      >
        <span class="sidebar-marquee-copy">{{ conversation.title }}</span>
        <span class="sidebar-marquee-copy" aria-hidden="true">{{ conversation.title }}</span>
      </span>
      <span v-else ref="titleText">{{ conversation.title }}</span>
    </span>
    <span
      class="absolute inset-y-0 right-1 flex items-center gap-0.5 transition-opacity"
      :class="
        menuOpen ? 'opacity-100' : 'opacity-0 group-hover/row:opacity-100 focus-within:opacity-100'
      "
    >
      <Tooltip :content="conversation.pinned ? 'Unpin' : 'Pin'" class="flex items-center">
        <IconButton
          v-if="!hidePin"
          :icon="conversation.pinned ? PinOff : Pin"
          :icon-size="ICON_PX.in20"
          size="sm"
          variant="ghost"
          :pressed="conversation.pinned"
          :aria-pressed="conversation.pinned"
          :aria-label="conversation.pinned ? 'Unpin conversation' : 'Pin conversation'"
          @click.stop="emit('togglePin')"
        />
      </Tooltip>
      <Tooltip content="Archive conversation" class="flex items-center">
        <IconButton
          :icon="Archive"
          size="sm"
          variant="ghost"
          aria-label="Archive conversation"
          :disabled="conversation.status === 'active'"
          @click.stop="emit('archive')"
        />
      </Tooltip>
    </span>
    <Pin
      v-if="conversation.pinned && !menuOpen"
      :size="ICON_PX.in20"
      class="pointer-events-none absolute right-[10px] text-fg-faint transition-opacity group-hover/row:opacity-0"
    />
  </div>
</template>

<style scoped>
.sidebar-breath {
  animation: sidebar-breath 2.4s ease-in-out infinite;
}

@keyframes sidebar-breath {
  0%,
  100% {
    opacity: 0.35;
    transform: scale(0.8);
  }
  50% {
    opacity: 1;
    transform: scale(1.15);
  }
}

/* One animated value drives both edge fades. */
@property --sidebar-edge-opacity {
  syntax: '<number>';
  inherits: false;
  initial-value: 1;
}

.sidebar-title {
  text-overflow: ellipsis;
  --sidebar-edge-opacity: 1;
  mask-image: linear-gradient(
    to right,
    rgb(0 0 0 / var(--sidebar-edge-opacity)),
    black 1rem,
    black calc(100% - 1rem),
    rgb(0 0 0 / var(--sidebar-edge-opacity))
  );
  transition:
    margin 80ms ease,
    --sidebar-edge-opacity 120ms ease;
}

.sidebar-title.is-playing {
  --sidebar-edge-opacity: 0;
}

.sidebar-marquee {
  will-change: transform;
  animation: sidebar-marquee var(--marquee-ms) linear infinite;
}

.sidebar-marquee-copy {
  flex-shrink: 0;
  padding-right: var(--marquee-gap);
}

/* Each half contains the same title and gap, so the loop boundary is seamless. */
@keyframes sidebar-marquee {
  from {
    transform: translateX(0);
  }
  to {
    transform: translateX(-50%);
  }
}

@media (prefers-reduced-motion: reduce) {
  .sidebar-title {
    transition: none;
  }
  .sidebar-breath {
    animation: none;
  }

  .sidebar-marquee {
    animation: none;
    transform: none;
  }
}
</style>
