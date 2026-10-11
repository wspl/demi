<script setup lang="ts">
import { computed } from 'vue'
import { Archive, Pin, PinOff } from '@lucide/vue'
import DeviceIcon from '@demicodes/web-ui/devices/DeviceIcon.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import TitleInput from '@demicodes/web-ui/ui/TitleInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { isTextCut } from '@demicodes/web-ui/ui/truncation'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import { DEVICE_STATE_LABEL } from '@demicodes/web-ui/devices/state'
import type { SidebarConversation } from './types'
import ConversationDot from './ConversationDot.vue'

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

// The dot's look is `ConversationDot`'s. An unread answer also shows in the title's emphasis.

// The device a conversation outside a project runs on, named with its state as a row names an
// item and its state (the Writing page's "A screen says a thing once").
const deviceLabel = computed(() => {
  const device = props.conversation.device
  if (!device) {
    return null
  }
  return device.kind === 'paired' ? `${device.name} · ${DEVICE_STATE_LABEL[device.state]}` : 'Removed device'
})

const phone = usePhone()

// The marks the row's end shows at rest, each in a 24px box: the device's, then the pin.
const restMarks = computed(() => (props.conversation.device ? 1 : 0) + (props.conversation.pinned ? 1 : 0))

// The title's end clears what the row's end shows: the marks at rest, each 24px with an 8px gap
// before them, and the actions on hover, Archive alone (24px) or Unpin and Archive on a pinned
// row (50px). Where the marks are wider than the actions, the title keeps their margin on hover
// rather than growing under the pointer. On a phone each box is 32px, and with no hover the open
// row, which a tap opened, shows the actions instead of the marks.
const titleMargin = computed(() => {
  const pinned = props.conversation.pinned
  if (props.menuOpen) {
    return pinned ? 'mr-[50px] phone:mr-[74px]' : 'mr-6 phone:mr-10'
  }
  const onPhone = props.open
    ? (pinned ? 'phone:mr-[74px]' : 'phone:mr-10')
    : ['phone:mr-0', 'phone:mr-10', 'phone:mr-[74px]'][restMarks.value]
  if (restMarks.value === 2) {
    return `mr-[58px] ${onPhone}`
  }
  if (restMarks.value === 1) {
    return `${pinned ? 'mr-8 group-hover/row:mr-[50px]' : 'mr-8'} ${onPhone}`
  }
  return `group-hover/row:mr-6 ${onPhone}`
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
    class="group/row relative flex h-7 cursor-default select-none items-center gap-2 rounded-md pl-2.5 pr-0.5 text-chrome transition-colors duration-200 ease-out phone:h-11 phone:gap-3 phone:rounded-lg phone:pl-3 phone:text-[16px]"
    :class="rowClass"
    :aria-selected="selected"
    @click="emit('click', $event)"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <ConversationDot :conversation="conversation" />
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
      :class="menuOpen ? 'opacity-100' : ['opacity-0 group-hover/row:opacity-100 focus-within:opacity-100', open ? 'phone:opacity-100' : '']"
    >
      <!-- A pinned row offers Unpin before Archive; Pin is in the row's menu
           (`product.md` § Conversations and projects). -->
      <Tooltip v-if="conversation.pinned" content="Unpin" class="flex items-center">
        <IconButton
          :icon="PinOff"
          :icon-size="phone ? ICON_PX.in32 : ICON_PX.in20"
          :size="phone ? 'lg' : 'sm'"
          variant="ghost"
          aria-label="Unpin"
          @click.stop="emit('unpin')"
        />
      </Tooltip>
      <Tooltip content="Archive" class="flex items-center">
        <IconButton
          :icon="Archive"
          :size="phone ? 'lg' : 'sm'"
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
      :class="open ? 'phone:opacity-0' : ''"
    >
      <span v-if="deviceLabel" class="flex size-6 items-center justify-center phone:size-8">
        <DeviceIcon
          :state="conversation.device?.kind === 'paired' ? conversation.device.state : null"
          :label="deviceLabel"
        />
      </span>
      <span v-if="conversation.pinned" class="flex size-6 items-center justify-center phone:size-8">
        <Pin :size="phone ? ICON_PX.in32 : ICON_PX.in20" />
      </span>
    </span>
  </div>
</template>
