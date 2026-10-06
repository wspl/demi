<script setup lang="ts">
import type { Component } from 'vue'
import Button from './Button.vue'
import type { SentenceText, TitleText } from './ui-text'

/**
 * A quiet line over a region whose content still shows, about that content:
 * a file view that could not refresh what it shows, or that shows files as
 * they were last read because the Host cannot watch them. It never replaces
 * the content, as `RegionStatus` does, and it offers at most one action.
 */
defineProps<{
  /** The glyph before the sentence. */
  icon: Component
  label: SentenceText
  /** The reason after the sentence, in the caller's words. */
  detail?: string | null
  /** The single action's label; omitted notes offer nothing. */
  action?: TitleText
}>()
defineEmits<{ action: [] }>()
</script>

<template>
  <!-- The action keeps the same distance to the note's end as to its top and bottom: (28 - 20) / 2. -->
  <div
    role="status"
    class="flex h-7 shrink-0 select-none items-center gap-2 border-b border-line pl-3 text-[12px] text-fg-muted"
    :class="action ? 'pr-1' : 'pr-3'"
  >
    <component :is="icon" :size="12" class="shrink-0 text-fg-subtle" aria-hidden="true" />
    <p class="min-w-0 flex-1 truncate" :title="detail ?? undefined">
      {{ label }}<span v-if="detail" class="ml-1 text-fg-subtle">{{ detail }}</span>
    </p>
    <Button v-if="action" size="xs" variant="ghost" class="shrink-0" @click="$emit('action')">{{ action }}</Button>
  </div>
</template>
