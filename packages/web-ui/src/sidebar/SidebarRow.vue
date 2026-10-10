<script setup lang="ts">
import { computed } from 'vue'
import { Archive, Pin, PinOff } from '@lucide/vue'
import DeviceIcon from '@demicodes/web-ui/devices/DeviceIcon.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import TitleInput from '@demicodes/web-ui/ui/TitleInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { isTextCut } from '@demicodes/web-ui/ui/truncation'
import { DEVICE_STATE_LABEL } from '@demicodes/web-ui/devices/state'
import type { SidebarConversation } from './types'

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
}>()

const emit = defineEmits<{
  click: [event: MouseEvent]
  contextmenu: [event: MouseEvent]
  archive: []
  /** A double-click on the title: the row names itself in place, as Rename in its menu does. */
  renameStart: []
  renameSubmit: [title: string]
  renameCancel: []
  unpin: []
}>()

// One quiet mark: yellow while a permission request waits for the user, over every other mark and
// whether the row is open or read; a breathing dot while running, blue for a result waiting to be
// read, orange when the conversation failed. A turn the user stopped is their own decision, not a
// failure, and leaves the row settled (`product.md` § Recovering an unfinished turn). A settled
// row keeps a faint ring in the dot's place, so the column under a project's icon is never empty.
const SETTLED_DOT = 'border border-fg-faint/60'

const dotClass = computed(() => {
  const { status, unread, needsYou } = props.conversation
  if (needsYou) {
    // The halo keeps it apart from the failed mark, whose amber is close to yellow in dark mode.
    return 'bg-on-attention ring-2 ring-on-attention/35'
  }
  if (status === 'active') {
    return 'sidebar-breath bg-fg'
  }
  if (props.open || !unread) {
    return SETTLED_DOT
  }
  if (status === 'error') {
    return 'bg-on-warning'
  }
  if (status === 'done' && unread) {
    return 'bg-on-info'
  }
  return SETTLED_DOT
})

// The device a conversation outside a project runs on, named with its state as a row names an
// item and its state (the Writing page's "A screen says a thing once").
const deviceLabel = computed(() => {
  const device = props.conversation.device
  if (!device) {
    return null
  }
  return device.kind === 'paired' ? `${device.name} · ${DEVICE_STATE_LABEL[device.state]}` : 'Removed device'
})

// The marks the row's end shows at rest, each in a 24px box: the device's, then the pin.
const restMarks = computed(() => (props.conversation.device ? 1 : 0) + (props.conversation.pinned ? 1 : 0))

// The title's end clears what the row's end shows: the marks at rest, each 24px with an 8px gap
// before them, and the actions on hover, Archive alone (24px) or Unpin and Archive on a pinned
// row (50px). Where the marks are wider than the actions, the title keeps their margin on hover
// rather than growing under the pointer.
const titleMargin = computed(() => {
  const pinned = props.conversation.pinned
  if (props.menuOpen) {
    return pinned ? 'mr-[50px]' : 'mr-6'
  }
  if (restMarks.value === 2) {
    return 'mr-[58px]'
  }
  if (restMarks.value === 1) {
    return pinned ? 'mr-8 group-hover/row:mr-[50px]' : 'mr-8'
  }
  return 'group-hover/row:mr-6'
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
</script>

<template>
  <div
    class="group/row relative flex h-7 cursor-default select-none items-center gap-2 rounded-md pl-2.5 pr-0.5 text-chrome transition-colors duration-200 ease-out"
    :class="rowClass"
    :aria-selected="selected"
    @click="emit('click', $event)"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <span class="flex size-3.5 shrink-0 items-center justify-center">
      <span
        class="size-1.5 rounded-full"
        :class="dotClass"
        :role="conversation.needsYou ? 'img' : undefined"
        :aria-label="conversation.needsYou ? 'Needs your decision' : undefined"
      />
    </span>
    <TitleInput
      v-if="renaming"
      class="flex-1"
      size="sm"
      :title="conversation.title"
      @submit="emit('renameSubmit', $event)"
      @cancel="emit('renameCancel')"
    />
    <!-- The title has the row until hover; then it yields the end to the actions. A cut title ends
         in an ellipsis and shows whole in its tooltip, as Finder and Mail show a long name; it
         never moves. It is clipped, not hidden: a hidden overflow still scrolls when focus or a
         scroll-into-view lands inside it, and the row would be left showing the title's tail. -->
    <Tooltip
      v-else
      :content="conversation.title"
      :show-if="isTextCut"
      placement="bottom-start"
      class="min-w-0 flex-1 overflow-clip text-ellipsis whitespace-nowrap transition-[margin] duration-[80ms] ease-out"
      :class="[conversation.unread && !open ? 'text-fg-emphasis' : '', titleMargin]"
      @dblclick="emit('renameStart')"
    >{{ conversation.title }}</Tooltip>
    <!-- While renaming the field has the whole row: no actions, no pin glyph over it. -->
    <span
      v-if="!renaming"
      class="on-fill absolute inset-y-0 right-0.5 flex items-center gap-0.5 transition-opacity"
      :class="menuOpen ? 'opacity-100' : 'opacity-0 group-hover/row:opacity-100 focus-within:opacity-100'"
    >
      <!-- A pinned row offers Unpin before Archive; Pin is in the row's menu
           (`product.md` § Conversations and projects). -->
      <Tooltip v-if="conversation.pinned" content="Unpin" class="flex items-center">
        <IconButton
          :icon="PinOff"
          :icon-size="ICON_PX.in20"
          size="sm"
          variant="ghost"
          aria-label="Unpin"
          @click.stop="emit('unpin')"
        />
      </Tooltip>
      <Tooltip content="Archive" class="flex items-center">
        <IconButton
          :icon="Archive"
          size="sm"
          variant="ghost"
          aria-label="Archive"
          :disabled="conversation.status === 'active'"
          @click.stop="emit('archive')"
        />
      </Tooltip>
    </span>
    <!-- The marks sit in the same 24px boxes as the actions: the device's mark, then the pinned
         glyph at the end, which Unpin takes one place left of on hover. -->
    <span
      v-if="restMarks && !menuOpen && !renaming"
      class="pointer-events-none absolute inset-y-0 right-0.5 flex items-center gap-0.5 text-fg-faint transition-opacity group-hover/row:opacity-0"
    >
      <span v-if="deviceLabel" class="flex size-6 items-center justify-center">
        <DeviceIcon
          :state="conversation.device?.kind === 'paired' ? conversation.device.state : null"
          :label="deviceLabel"
        />
      </span>
      <span v-if="conversation.pinned" class="flex size-6 items-center justify-center">
        <Pin :size="ICON_PX.in20" />
      </span>
    </span>
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
</style>
