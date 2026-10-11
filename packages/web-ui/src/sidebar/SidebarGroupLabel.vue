<script setup lang="ts">
import FoldChevron from '@demicodes/web-ui/ui/FoldChevron.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { useTouchOnly } from '@demicodes/web-ui/ui/touch-only'
import ConversationDot from './ConversationDot.vue'
import type { SidebarConversation } from './types'

/**
 * A group's label, Conversations or Projects, which folds the group under it as Finder's
 * sidebar sections do (`product.md` § Conversations and projects): the label and the chevron
 * after it are one button. The chevron, in the label's faint color, shows while the pointer is
 * over the label or the keyboard is on the button, and always on a touch screen, which has no
 * hover. While the group is folded, a conversation in it that waits for the user puts its dot
 * after the chevron. The `action` slot holds the group's button at the label's end, such as New
 * conversation, which shows on hover.
 */
defineProps<{
  label: string
  folded: boolean
  /** The conversation whose dot the label carries; none while nothing in the folded group waits. */
  waiting: Pick<SidebarConversation, 'status' | 'needsYou'> | null
}>()

const emit = defineEmits<{
  toggle: []
}>()

const touchOnly = useTouchOnly()
</script>

<template>
  <div
    class="group/heading mb-1.5 flex h-6 items-center pr-0.5 text-[11px] uppercase tracking-wide text-fg-subtle"
  >
    <!-- The button starts where the rows' content does, so the label stays on the list's line. -->
    <button
      type="button"
      class="group/fold flex h-full min-w-0 items-center gap-1 rounded-md pl-2.5 pr-1 uppercase"
      :aria-expanded="!folded"
      @click="emit('toggle')"
    >
      <span class="truncate">{{ label }}</span>
      <span
        class="flex items-center transition-opacity duration-150 motion-reduce:transition-none"
        :class="touchOnly ? '' : 'opacity-0 group-hover/heading:opacity-100 group-focus-visible/fold:opacity-100'"
      >
        <FoldChevron :open="!folded" :size="ICON_PX.in20" />
      </span>
      <ConversationDot v-if="waiting" :conversation="waiting" />
    </button>
    <span class="flex-1" />
    <span
      class="flex items-center opacity-0 transition-opacity group-hover/heading:opacity-100 focus-within:opacity-100"
    >
      <slot name="action" />
    </span>
  </div>
</template>
