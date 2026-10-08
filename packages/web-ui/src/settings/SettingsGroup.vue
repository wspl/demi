<script setup lang="ts">
import type { SentenceText, TitleText } from '../ui/ui-text'
/**
 * A titled card of rows. Rows separate themselves with hairlines. An
 * `actions` slot puts buttons at the end of the title's line, such as Add
 * Device…, centred on it. A `header` slot replaces the title when the group's
 * subject needs its own controls, such as an editable name beside its
 * actions; an `aside` slot puts something beside the card, such as a live
 * preview of what the rows change. A `footer` slot is the line macOS writes
 * under a group: small, secondary, about what the rows above decide.
 *
 * A `plain` group is text under its title rather than a card of rows: a
 * section that says how something fares, such as a device's P2P
 * Connection; without content it is its title alone.
 */
defineProps<{
  title?: TitleText
  description?: SentenceText
  plain?: boolean
}>()
</script>

<template>
  <!-- The group is the container its aside is placed by: the page's width, not the dialog's. -->
  <section class="@container flex flex-col" :class="plain ? 'gap-1' : 'gap-3'">
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
    <div v-if="plain" class="text-[13px] leading-5 text-fg-muted">
      <slot />
    </div>
    <!-- An aside (a preview, a legend) sits beside the card where there is room, under it where not. -->
    <div v-else class="@container flex flex-col gap-4 @lg:flex-row @lg:items-stretch">
      <div class="flex min-w-0 flex-1 flex-col gap-1.5">
        <div
          class="settings-card @container min-w-0 flex-1 overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <slot />
        </div>
        <!-- The footnote starts where the rows' text starts. -->
        <p v-if="$slots.footer" class="select-none px-4 text-[12px] leading-4 text-fg-muted">
          <slot name="footer" />
        </p>
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
