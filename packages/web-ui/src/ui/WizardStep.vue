<script setup lang="ts">
import type { Component } from 'vue'
import type { HeadlineText, SentenceText } from './ui-text'

/**
 * One step of a `Wizard`: what it asks as a headline and a sentence, the body that
 * answers it, and the step's actions at the foot.
 *
 * - On a computer the dialog is a headline and a sentence at the leading edge with
 *   the actions at the trailing edge, the default one last.
 * - On a phone the step is the whole screen as iOS's setup screens are: a large
 *   symbol centred in the upper third, the title and the sentence centred under
 *   it, and the actions at the bottom, a finger tall and as wide as the screen,
 *   the default one first. The symbol shows only there.
 */
defineProps<{
  title: HeadlineText
  message?: SentenceText
  /** The symbol a phone's screen shows above the title. */
  icon?: Component
}>()

defineSlots<{
  /** What answers the step: a field, a failure. */
  default?(): unknown
  /** The step's buttons, written as `Button`s; a phone makes them full width. */
  actions(): unknown
}>()
</script>

<template>
  <div class="flex flex-1 flex-col gap-4 p-6 pt-8 @max-3xl:px-8 @max-3xl:pt-[14vh]">
    <component
      :is="icon"
      v-if="icon"
      :size="64"
      :stroke-width="1.25"
      class="hidden self-center text-accent-fill @max-3xl:block"
      aria-hidden="true"
    />
    <header class="select-none pr-8 @max-3xl:pr-0 @max-3xl:text-center">
      <h2 class="text-[17px] font-medium leading-6 text-fg-emphasis @max-3xl:text-[26px] @max-3xl:leading-8">{{ title }}</h2>
      <p v-if="message" class="mt-1 text-[13px] leading-5 text-fg-muted @max-3xl:mt-3 @max-3xl:text-[16px] @max-3xl:leading-6">{{ message }}</p>
    </header>
    <div v-if="$slots.default" class="flex flex-col gap-2"><slot /></div>
    <!-- On a phone the actions stand at the screen's foot, the default one first, each a full-width row. -->
    <div
      class="mt-auto flex items-center justify-end gap-2 [&>:has([data-default-action])]:order-last @max-3xl:flex-col-reverse @max-3xl:items-stretch @max-3xl:[&>*]:h-11 @max-3xl:[&>*]:w-full @max-3xl:[&>*]:text-[15px]"
    >
      <slot name="actions" />
    </div>
  </div>
</template>
