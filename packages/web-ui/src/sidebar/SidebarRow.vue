<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { Archive, Pin, PinOff } from '@lucide/vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import TitleInput from '@demicodes/web-ui/ui/TitleInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { isTextCut } from '@demicodes/web-ui/ui/truncation'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import type { SidebarConversation } from './types'
import ConversationDot from './ConversationDot.vue'
import DeviceMark from './DeviceMark.vue'
import { conversationTitle } from '../agent/conversation-title'

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

const phone = usePhone()
const shownTitle = computed(() => conversationTitle(props.conversation.title))

// The row's end holds, in the flow, the pin's box and the device's mark, 24px each (32px on a
// phone) with 2px between. The actions stand over them: Archive over the pin's box, or just left
// of the mark when the row is not pinned, and Unpin left of Archive. The title clears them with
// one margin, since the pin's box and the mark already hold the title back: Archive alone (24px,
// 32px on a phone) or Unpin before it. On a phone, with no hover, the open row, which a tap
// opened, shows the actions; the mark stays.
const titleMargin = computed(() => {
  if (props.menuOpen) {
    return 'mr-6 phone:mr-8'
  }
  return props.open ? 'group-hover/row:mr-6 phone:mr-8' : 'group-hover/row:mr-6 phone:mr-0'
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
  props.focused ? 'focus-ring-shown' : '',
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
      :title="shownTitle"
      @submit="emit('renameSubmit', $event)"
      @cancel="emit('renameCancel')"
    />
    <!-- The title has the row until hover; then it yields the end to the actions. A cut title ends
         in an ellipsis and shows whole in its tooltip, as Finder and Mail show a long name; it
         never moves. It is clipped, not hidden: a hidden overflow still scrolls when focus or a
         scroll-into-view lands inside it, and the row would be left showing the title's tail. -->
    <Tooltip
      v-else
      :content="shownTitle"
      :show-if="isTextCut"
      placement="bottom-start"
      class="min-w-0 flex-1 overflow-clip text-ellipsis whitespace-nowrap transition-[margin] duration-[80ms] ease-out"
      :class="[conversation.unread && !open ? 'text-fg-emphasis' : '', titleMargin]"
      @dblclick="emit('renameStart')"
    >{{ shownTitle }}</Tooltip>
    <!-- The pin's glyph and the device's mark end the row, in the flow, so the title never reaches
         them. The mark stays on hover; the pin gives its place to Archive. -->
    <span v-if="!renaming && (conversation.pinned || conversation.device)" class="flex shrink-0 items-center gap-0.5">
      <span
        v-if="conversation.pinned"
        class="pointer-events-none flex size-6 items-center justify-center text-fg-faint transition-opacity phone:size-8"
        :class="menuOpen ? 'opacity-0' : ['group-hover/row:opacity-0', open ? 'phone:opacity-0' : '']"
      >
        <Pin :size="phone ? ICON_PX.in32 : ICON_PX.in20" />
      </span>
      <DeviceMark v-if="conversation.device" :host="conversation.device" />
    </span>
    <!-- While renaming the field has the whole row: no actions, no pin glyph over it. The actions
         stand where the pin's box is, and left of the device's mark when the row is not pinned. -->
    <span
      v-if="!renaming"
      class="on-fill absolute inset-y-0 flex items-center gap-0.5 transition-opacity"
      :class="[
        conversation.device ? 'right-[calc(var(--spacing-hit-sm)+4px)] phone:right-9' : 'right-0.5',
        menuOpen ? 'opacity-100' : ['opacity-0 group-hover/row:opacity-100 focus-within:opacity-100', open ? 'phone:opacity-100' : ''],
      ]"
    >
      <!-- A pinned row offers Unpin before Archive; Pin is in the row's menu
           (`conversations.md` § Conversations and projects). -->
      <Tooltip v-if="conversation.pinned" :content="t('row.unpin')" class="flex items-center">
        <IconButton
          :icon="PinOff"
          :icon-size="phone ? ICON_PX.in32 : ICON_PX.in20"
          :size="phone ? 'lg' : 'sm'"
          variant="ghost"
          :aria-label="t('row.unpin')"
          @click.stop="emit('unpin')"
        />
      </Tooltip>
      <Tooltip :content="t('row.archive')" class="flex items-center">
        <IconButton
          :icon="Archive"
          :size="phone ? 'lg' : 'sm'"
          variant="ghost"
          :aria-label="t('row.archive')"
          :disabled="conversation.status === 'active'"
          @click.stop="emit('archive')"
        />
      </Tooltip>
    </span>
  </div>
</template>
