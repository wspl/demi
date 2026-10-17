<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { Archive, Pin, PinOff, Zap } from '@lucide/vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import TitleInput from '@demicodes/web-ui/ui/TitleInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { isTextCut } from '@demicodes/web-ui/ui/truncation'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import type { SidebarConversation } from './types'
import ConversationDot from './ConversationDot.vue'
import DeviceMark from './DeviceMark.vue'
import { t as automationText } from '../automations/locales/text'
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
// one margin, since the pin's box and the mark already hold the title back: Archive alone (24px)
// or Unpin before it. A phone row shows no actions, since a tap meant to open the row could land
// on one; its long press opens its menu, which holds them (`web-application.md` § The phone
// layout), so its title and pin stay put.
const titleMargin = computed(() => {
  if (phone.value) {
    return ''
  }
  return props.menuOpen ? 'mr-6' : 'group-hover/row:mr-6'
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
    class="group/row relative flex h-7 cursor-default select-none items-center gap-2 rounded-md pl-2.5 pr-0.5 text-chrome transition-colors duration-200 ease-out phone:h-11 phone:gap-3 phone:rounded-lg phone:pl-3 phone:text-[16px]"
    :class="rowClass"
    :aria-selected="selected"
    @click="emit('click', $event)"
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
    <span v-if="!renaming && (conversation.pinned || conversation.device || conversation.automations?.length)" class="flex shrink-0 items-center gap-0.5">
      <span
        v-if="conversation.pinned"
        class="pointer-events-none flex size-6 items-center justify-center text-fg-faint transition-opacity phone:size-8"
        :class="pinClass"
      >
        <Pin :size="phone ? ICON_PX.in32 : ICON_PX.in20" />
      </span>
      <!-- The bolt before the device's mark says automations wake the conversation; a mark only, the row opens the conversation. -->
      <span
        v-if="conversation.automations?.length"
        role="img"
        :aria-label="automationText('mark.bolt')"
        class="flex size-hit-sm shrink-0 items-center justify-center text-fg-subtle phone:size-8"
      >
        <Zap :size="phone ? ICON_PX.in32 : ICON_PX.in20" />
      </span>
      <DeviceMark v-if="conversation.device" :host="conversation.device" />
    </span>
    <!-- While renaming the field has the whole row: no actions, no pin glyph over it. The actions
         stand where the pin's box is, and left of the device's mark when the row is not pinned. -->
    <span
      v-if="!renaming && !phone"
      class="on-fill absolute inset-y-0 flex items-center gap-0.5 transition-opacity"
      :class="[
        conversation.device && conversation.automations?.length
          ? 'right-[calc(2*var(--spacing-hit-sm)+6px)]'
          : conversation.device || conversation.automations?.length ? 'right-[calc(var(--spacing-hit-sm)+4px)]' : 'right-0.5',
        menuOpen ? 'opacity-100' : 'opacity-0 group-hover/row:opacity-100 focus-within:opacity-100',
      ]"
    >
      <!-- A pinned row offers Unpin before Archive; Pin is in the row's menu
           (`conversations.md` § Conversations and projects). -->
      <Tooltip v-if="conversation.pinned" :content="t('row.unpin')" class="flex items-center">
        <IconButton
          :icon="PinOff"
          :icon-size="ICON_PX.in20"
          size="sm"
          variant="ghost"
          :aria-label="t('row.unpin')"
          @click.stop="emit('unpin')"
        />
      </Tooltip>
      <Tooltip :content="t('row.archive')" class="flex items-center">
        <IconButton
          :icon="Archive"
          size="sm"
          variant="ghost"
          :aria-label="t('row.archive')"
          :disabled="conversation.status === 'active'"
          @click.stop="emit('archive')"
        />
      </Tooltip>
    </span>
  </div>
</template>
