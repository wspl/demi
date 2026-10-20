<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import StatusDot from '@demicodes/web-ui/ui/StatusDot.vue'
import { conversationDotTone } from './conversation-dot'
import type { SidebarConversation } from './types'

// One quiet mark: yellow while a permission request waits for the user and has
// not been read, the same size as every other dot and with nothing around it,
// over every other mark. Otherwise it is the tone the conversation's state wears
// on its tab too: the accent breathing while it runs, green once finished and
// red once failed, until the user reads it.
//
// A read answer, failure or request, a turn the user stopped (their own
// decision), and a conversation that ran nothing leave the row settled: a faint
// ring in the dot's place, so the column under a project's icon is never empty.
//
// A folded group's label carries the dot of a conversation in it that waits, so
// the mark is drawn here alone.
const props = defineProps<{
  conversation: Pick<SidebarConversation, 'status' | 'needsYou' | 'unread'>
}>()

const tone = computed(() => conversationDotTone(props.conversation))
</script>

<template>
  <span class="flex size-3.5 shrink-0 items-center justify-center phone:size-4 phone:[&>*]:scale-[1.35]">
    <StatusDot v-if="tone === 'attention'" tone="attention" :label="t('dot.needsYou')" />
    <StatusDot v-else-if="tone" :tone="tone" :breathing="conversation.status === 'active'" />
    <StatusDot v-else tone="muted" ring />
  </span>
</template>
