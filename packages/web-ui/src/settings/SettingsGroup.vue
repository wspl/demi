<script setup lang="ts">
import type { SentenceText, TitleText } from '../ui/ui-text'
/**
 * A titled card of rows. Rows separate themselves with hairlines. An
 * `actions` slot puts buttons at the end of the title's line, such as Add
 * Device…, centred on it. A `header` slot replaces the title when the group's
 * subject needs its own controls, such as an editable name beside its
 * actions; an `aside` slot puts something beside the card, such as a live
 * preview of what the rows change.
 */
defineProps<{
  title?: TitleText
  description?: SentenceText
}>()
</script>

<template>
  <!-- The group is the container its aside is placed by: the page's width, not the dialog's. -->
  <section class="@container flex flex-col gap-3">
    <slot name="header">
      <header v-if="title" class="select-none">
        <!-- With buttons, the title's line is as tall as they are, and they are centred on it. -->
        <div class="flex items-center justify-between gap-3" :class="$slots.actions ? 'min-h-7' : ''">
          <h3 class="min-w-0 text-[15px] font-medium leading-5 text-fg-emphasis">{{ title }}</h3>
          <div v-if="$slots.actions" class="flex shrink-0 items-center gap-2">
            <slot name="actions" />
          </div>
        </div>
        <p
          v-if="description"
          class="mt-0.5 text-[13px] leading-5 text-fg-muted"
        >{{ description }}</p>
      </header>
    </slot>
    <!-- An aside (a preview, a legend) sits beside the card where there is room, under it where not. -->
    <div class="@container flex flex-col gap-4 @lg:flex-row @lg:items-stretch">
      <div
        class="settings-card @container min-w-0 flex-1 overflow-hidden rounded-xl border border-line bg-surface-float"
      >
        <slot />
      </div>
      <div v-if="$slots.aside" class="flex shrink-0">
        <slot name="aside" />
      </div>
    </div>
  </section>
</template>

<style>
.settings-card > * + * {
  border-top: 1px solid var(--line-subtle);
}
</style>
