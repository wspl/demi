<script setup lang="ts">
import { reactive } from 'vue'
import SelectionFieldQuote from '@demicodes/web-ui/agent/comments/SelectionFieldQuote.vue'
import SelectionField from '@demicodes/web-ui/agent/comments/SelectionField.vue'
import { t } from '@demicodes/web-ui/agent/locales/text'
import { productWould } from '../product-would'

/**
 * The comment field pinned in the states it has, each live: its own words,
 * every action saying what the product would do with the selection, and ✕
 * closing it, whereupon it comes back so it can be tried again. A press of
 * Comment, Ask or Return with no words shakes the field once.
 */
const states = reactive([
  { label: 'No words: Send is dimmed', text: '', way: 'comment' as const },
  { label: 'With words', text: 'Is 4 MB right for a slow link?', way: 'comment' as const },
  { label: 'Ask chosen from the menu: Return asks', text: 'Why 4 MB?', way: 'ask' as const },
  { label: 'The reply is still being written', text: 'Is 4 MB right for a slow link?', way: 'comment' as const, unavailable: true },
  { label: 'On a phone, in the bottom sheet without a scrim, quoting the selection in one line', text: '', way: 'comment' as const, phone: true, quote: 'The upload request runs under the same 30-second timeout as every other API call, so a 40 MB video on a 3G connection is cut off part of the way through' },
  { label: 'On a phone, a picture’s comment quotes the media text alone', text: '', way: 'comment' as const, phone: true, quote: t('comments.media') },
])
const shown = reactive(states.map(() => true))

function close(index: number): void {
  shown[index] = false
  productWould('Close the Field and Keep the Selection')
  setTimeout(() => (shown[index] = true), 600)
}
</script>

<template>
  <div class="flex flex-wrap items-start gap-8">
    <figure v-for="(state, index) in states" :key="state.label" class="flex flex-col gap-2" :class="state.phone ? 'w-[377px]' : ''">
      <!-- On a phone the field sits in the bottom sheet, which has no scrim, here drawn as the sheet holds it. -->
      <div :class="state.phone ? 'rounded-2xl bg-surface-dialog px-3 pb-3 shadow-2xl' : ''">
        <div v-if="state.phone" class="flex h-5 items-center justify-center"><span class="h-1 w-9 rounded-full bg-fg-faint opacity-60" /></div>
        <SelectionFieldQuote v-if="state.phone" :quote="state.quote" @close="close(index)" />
        <SelectionField
          v-if="shown[index]"
          v-model="state.text"
          ask
          send
          :way="state.way"
          :phone="state.phone"
          :unavailable="state.unavailable ? t('comments.waitReply') : null"
          @comment="productWould('Add the Words as a Comment on the Selection')"
          @ask="productWould('Ask the Words as a Side Question About the Selection')"
          @send="productWould('Send the Quote and the Words to the Main Agent Now')"
          @close="close(index)"
        />
      </div>
      <figcaption class="text-[12px] text-fg-subtle">{{ state.label }}</figcaption>
    </figure>
  </div>
</template>
