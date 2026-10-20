<script setup lang="ts">
import { t } from './locales/text'
import type { Component } from 'vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { HeadlineText } from '../ui/ui-text'

/**
 * The card every request of a conversation stands in, pinned above the composer. It
 * holds, in order:
 *
 * - a mark and the title, which one of how many requests it is while several wait,
 * - the request's own body, and
 * - its buttons.
 *
 * It is not modal: the transcript and the composer stay usable.
 */
defineProps<{
  title: HeadlineText
  icon: Component
  /** How many requests wait, this one included. */
  count: number
}>()
</script>

<template>
  <section
    class="permission-card overlay-window flex w-full flex-col gap-2.5 rounded-lg bg-surface-raised p-3 text-chrome"
    role="alertdialog"
    :aria-label="title"
  >
    <header class="flex items-start gap-2">
      <component
        :is="icon"
        :size="ICON_PX.in32"
        class="mt-px shrink-0 text-on-attention"
        aria-hidden="true"
      />
      <h3 class="min-w-0 flex-1 font-medium text-fg-emphasis">
        {{ title }}
      </h3>
      <span v-if="count > 1" class="shrink-0 tabular-nums text-fg-subtle">{{ t('request.progress', { position: 1, count }) }}</span>
    </header>
    <slot />
    <footer class="flex flex-wrap justify-end gap-2">
      <slot name="footer" />
    </footer>
  </section>
</template>
