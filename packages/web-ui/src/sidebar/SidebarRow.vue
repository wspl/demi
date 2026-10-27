<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { AlarmClock, Archive, Pin, PinOff } from '@lucide/vue'
import WorktreeIcon from '@demicodes/web-ui/ui/WorktreeIcon.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import TitleInput from '@demicodes/web-ui/ui/TitleInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { isTextCut } from '@demicodes/web-ui/ui/truncation'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import { vContextMenu, type ContextMenuDeclaration } from '@demicodes/web-ui/ui/context-menu'
import type { SidebarConversation } from './types'
import ConversationDot from './ConversationDot.vue'
import DeviceMark from './DeviceMark.vue'
import { t as scheduleText } from '../schedules/locales/text'
import { t as hostText } from '../hosts/locales/text'
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
  /** The menu of the conversation by its id. */
  menu: (id: string) => ContextMenuDeclaration
}>()

// Each event names the row's conversation, so the list binds the same
// handler to every row, and a row whose props stay as they were is not drawn
// again when the list is.
const emit = defineEmits<{
  click: [id: string, event: MouseEvent]
  archive: [id: string]
  /** A double-click on the title: the row names itself in place, as Rename in its menu does. */
  renameStart: [id: string]
  renameSubmit: [id: string, title: string]
  renameCancel: []
  unpin: [id: string]
}>()

// The dot's look is `ConversationDot`'s. An unread answer also shows in the title's emphasis.

const phone = usePhone()
const shownTitle = computed(() => conversationTitle(props.conversation.title))

// The row's end holds, in the flow, the marks: the pin, the alarm clock and the device's
// mark, each in a box as wide as its glyph (14, 14 and 16px), 6px from the row's
// end. They are marks, not buttons. Every glyph at the row's end, mark or button,
// stands 6px from the next, as two side-by-side 20px buttons' 14px glyphs do, so
// the spacing does not change where the buttons meet the marks, and a row with
// several marks keeps most of its title.
//
// Marks and buttons rest in one grey, the ghost button's resting `text-fg-muted`,
// whatever color the row's text takes. Only the button under the pointer darkens
// and shows its fill, as Finder's eject button does, and a mark never changes on
// hover.
//
// The actions are 20px buttons, side by side, whose 14px glyphs sit 3px inside
// them:
//
// - Archive stands over the pin's glyph, or with its glyph 6px left of the
//   first mark when the row is not pinned.
// - Unpin stands left of Archive.
// - The title clears the actions by a margin that leaves 12px before the first
//   button's glyph.
//
// A phone row shows no actions, since a tap meant to open the row could land on
// one. Its long press opens its menu, which holds them, so its title and pin stay
// put, and its marks keep their 32px boxes.
const MARK_GAP_PX = 6
const END_INSET_PX = 6
/** An extra-small icon button (`IconButton` `xs`) is 20px with its glyph centered. */
const BUTTON_INSET_PX = (20 - ICON_PX.in24) / 2

/** The widths of the marks that stay on hover, the device's, the worktree's and the alarm clock's, from the row's end. */
const stayingMarks = computed(() => [
  ...(props.conversation.device ? [ICON_PX.markIn28] : []),
  ...(props.conversation.worktree ? [ICON_PX.in24] : []),
  ...(props.conversation.scheduled ? [ICON_PX.in24] : []),
])

/** Where the actions' right edge stands, from the row's: Archive's glyph where the pin's is. */
const actionsRight = computed(() =>
  stayingMarks.value.reduce((right, width) => right + width + MARK_GAP_PX, END_INSET_PX - BUTTON_INSET_PX))

const titleMargin = computed(() => {
  if (phone.value) {
    return ''
  }
  // Pinned: the pin holds the title back by Archive's glyph; Unpin's glyph, the gap before
  // Archive's and 12px remain, less the title's own 8px gap. Not pinned: the first mark
  // holds it back, and Archive's glyph, the gap and the 12px remain the same way; with no
  // mark only the row's end does.
  const margin = props.conversation.pinned || stayingMarks.value.length
    ? { hover: 'group-hover/row:mr-6', menu: 'mr-6' }
    : { hover: 'group-hover/row:mr-[26px]', menu: 'mr-[26px]' }
  return props.menuOpen ? margin.menu : margin.hover
})

const pinClass = computed(() => {
  if (phone.value) {
    return ''
  }
  return props.menuOpen ? 'opacity-0' : 'group-hover/row:opacity-0'
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
    class="group/row relative flex h-7 cursor-default select-none items-center gap-2 rounded-md pl-2.5 pr-1.5 text-chrome transition-colors duration-200 ease-out phone:h-11 phone:gap-3 phone:rounded-lg phone:pl-3 phone:pr-0.5 phone:text-[16px]"
    :class="rowClass"
    :aria-selected="selected"
    v-context-menu="menu(conversation.id)"
    @click="emit('click', conversation.id, $event)"
  >
    <ConversationDot :conversation="conversation" />
    <TitleInput
      v-if="renaming"
      class="flex-1"
      size="sm"
      :title="shownTitle"
      @submit="emit('renameSubmit', conversation.id, $event)"
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
      @dblclick="emit('renameStart', conversation.id)"
    >{{ shownTitle }}</Tooltip>
    <!-- The pin's glyph, the alarm clock, the worktree's mark and the device's mark end the row, in the flow, so the title never
         reaches them. The marks stay on hover; the pin gives its place to Archive. -->
    <span v-if="!renaming && (conversation.pinned || conversation.device || conversation.scheduled || conversation.worktree)" class="flex shrink-0 items-center gap-1.5 phone:gap-0.5">
      <span
        v-if="conversation.pinned"
        class="pointer-events-none flex h-6 w-3.5 items-center justify-center text-fg-muted transition-opacity phone:size-8"
        :class="pinClass"
      >
        <Pin :size="phone ? ICON_PX.in32 : ICON_PX.in24" />
      </span>
      <!-- The alarm clock before the device's mark says schedules wake the conversation; a mark only, the row opens the conversation. -->
      <span
        v-if="conversation.scheduled"
        role="img"
        :aria-label="scheduleText('mark')"
        class="flex h-6 w-3.5 shrink-0 items-center justify-center text-fg-muted phone:size-8"
      >
        <AlarmClock :size="phone ? ICON_PX.in32 : ICON_PX.in24" />
      </span>
      <!-- The worktree's mark says it works in a linked worktree of its project, which its tip names; a mark only. -->
      <Tooltip
        v-if="conversation.worktree"
        :content="hostText('directory.worktree', { name: conversation.worktree })"
        tag="span"
        class="flex h-6 w-3.5 shrink-0 items-center justify-center text-fg-muted phone:size-8"
      >
        <WorktreeIcon
          role="img"
          :aria-label="hostText('directory.worktree', { name: conversation.worktree })"
          :size="phone ? ICON_PX.in32 : ICON_PX.in24"
        />
      </Tooltip>
      <DeviceMark v-if="conversation.device" :host="conversation.device" />
    </span>
    <!-- While renaming the field has the whole row: no actions, no pin glyph over it. The actions
         stand where the pin is, and left of the alarm clock and the device's mark when the row is not pinned. -->
    <span
      v-if="!renaming && !phone"
      class="on-fill absolute inset-y-0 flex items-center transition-opacity"
      :class="menuOpen ? 'opacity-100' : 'opacity-0 group-hover/row:opacity-100 focus-within:opacity-100'"
      :style="{ right: `${actionsRight}px` }"
    >
      <!-- A pinned row offers Unpin before Archive; Pin is in the row's menu. -->
      <Tooltip v-if="conversation.pinned" :content="t('row.unpin')" class="flex items-center">
        <IconButton
          :icon="PinOff"
          size="xs"
          :icon-size="ICON_PX.in24"
          variant="ghost"
          :aria-label="t('row.unpin')"
          @click.stop="emit('unpin', conversation.id)"
        />
      </Tooltip>
      <Tooltip :content="t('row.archive')" class="flex items-center">
        <IconButton
          :icon="Archive"
          size="xs"
          :icon-size="ICON_PX.in24"
          variant="ghost"
          :aria-label="t('row.archive')"
          :disabled="conversation.status === 'active'"
          @click.stop="emit('archive', conversation.id)"
        />
      </Tooltip>
    </span>
  </div>
</template>
