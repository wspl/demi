<script setup lang="ts">
import FoldChevron from '@demicodes/web-ui/ui/FoldChevron.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import CapCentered from '@demicodes/web-ui/ui/CapCentered.vue'
import ConversationDot from './ConversationDot.vue'
import type { SidebarConversation } from './types'

/**
 * A group's label, Conversations or Projects, which folds the group under it as
 * Finder's sidebar sections do: the label and the chevron after it are one
 * button.
 *
 * - The chevron, in the label's faint color, shows while the pointer is over the
 *   label or the keyboard is on the button, and always on a phone, which has no
 *   hover.
 * - While the group is folded, the most urgent news of the conversations the
 *   fold hides, a failure, a request or a finished turn not read yet, puts its
 *   dot between the label and the chevron, so the chevron, hidden at rest,
 *   leaves no gap. A folded label keeps the dot's room while nothing is new, so
 *   news that comes or goes moves nothing.
 * - The `action` slot holds the group's button at the label's end, such as New
 *   conversation, which shows on hover.
 */
defineProps<{
  label: string
  folded: boolean
  /** The conversation whose dot the label carries; none while the group is open or nothing it hides is new. */
  news: Pick<SidebarConversation, 'status' | 'needsYou' | 'unread'> | null
}>()

const emit = defineEmits<{
  toggle: []
}>()
</script>

<template>
  <div
    class="group/heading mb-1.5 flex h-6 items-center pr-0.5 text-[11px] uppercase tracking-wide text-fg-subtle phone:h-11 phone:text-[13px]"
  >
    <!-- The button starts where the rows' content does, so the label stays on the list's line. -->
    <button
      type="button"
      class="group/fold flex h-full min-w-0 items-center rounded-md pl-2.5 pr-1 uppercase"
      :aria-expanded="!folded"
      @click="emit('toggle')"
    >
      <!-- The dot and the chevron sit on the middle of the label's capitals. -->
      <span class="flex min-w-0 items-baseline gap-1">
        <span class="truncate">{{ label }}</span>
        <CapCentered v-if="folded">
          <span class="flex size-3.5 shrink-0 phone:size-4">
            <ConversationDot v-if="news" :conversation="news" />
          </span>
        </CapCentered>
        <CapCentered
          class="opacity-0 transition-opacity duration-150 motion-reduce:transition-none group-hover/heading:opacity-100 group-focus-visible/fold:opacity-100 phone:opacity-100"
        >
          <FoldChevron :open="!folded" :size="ICON_PX.in20" />
        </CapCentered>
      </span>
    </button>
    <span class="flex-1" />
    <span
      class="flex items-center opacity-0 transition-opacity group-hover/heading:opacity-100 focus-within:opacity-100 phone:opacity-100"
    >
      <slot name="action" />
    </span>
  </div>
</template>
