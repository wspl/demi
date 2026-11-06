<script setup lang="ts">
import SuggestedReplies from '@demicodes/web-ui/agent/blocks/SuggestedReplies.vue'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { CHOICE_SUGGESTIONS, LONG_SUGGESTIONS, SUGGESTION_ROUNDS } from '../fixtures/suggested-replies'

/**
 * The row of suggested replies in each state it has, under an answer's
 * width: one, three long ones wrapped, three, being written and none, the
 * last two keeping a line's height. A pill's click and its Edit say in a
 * toast what the product would do with the words.
 */
const STATES: readonly { name: string; replies: readonly string[] | null; narrow?: boolean }[] = [
  { name: 'one', replies: SUGGESTION_ROUNDS[0] ?? [] },
  { name: 'three long ones in a narrow column, wrapped', replies: LONG_SUGGESTIONS, narrow: true },
  { name: 'three, one for each option the answer names', replies: CHOICE_SUGGESTIONS },
  { name: 'being written: the row keeps its height', replies: null },
  { name: 'none came: the row keeps its height', replies: [] },
]

function sent(text: string): void {
  showToast({ title: 'Message Sent', message: `The product sends “${text}” as a new message.`, tone: 'neutral' })
}

function edited(text: string): void {
  showToast({ title: 'Put into the Composer', message: `The product adds “${text}” to the composer’s draft and gives it the focus.`, tone: 'neutral' })
}
</script>

<template>
  <div class="flex flex-col gap-4">
    <div v-for="state in STATES" :key="state.name">
      <p class="mb-1 font-mono text-caption text-fg-faint">{{ state.name }}</p>
      <div class="rounded-lg border border-dashed border-line px-[var(--agent-pad-x,2rem)] pb-2" :class="state.narrow ? 'max-w-[26rem]' : ''">
        <SuggestedReplies :replies="state.replies" @send="sent" @edit="edited" />
      </div>
    </div>
  </div>
</template>
