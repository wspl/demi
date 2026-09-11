<script setup lang="ts">
import type { Component } from 'vue'
import { CircleX } from '@lucide/vue'
import Button from './Button.vue'
import IndeterminateSpinner from './IndeterminateSpinner.vue'

/**
 * The pane that stands in for a region whose content cannot be shown yet:
 * a loading list, a failed restore, an empty conversation, a missing id.
 *
 * Every region uses this one layout, so a failure reads the same in the
 * session pane, a settings list, the sidebar and a dialog body: a glyph for
 * the kind (a spinner while loading), one sentence, an optional detail line,
 * and at most one action. Retry returns the region to its loading state; there
 * is no third state between the two. A failed region announces itself
 * (`role="alert"`); loading and empty regions stay polite.
 */
withDefaults(
  defineProps<{
    /** Shown with a spinner; the region is still loading. */
    busy?: boolean
    /** The region cannot show its content because of a failure. */
    failed?: boolean
    /** The glyph above the sentence; a failure defaults to the error mark. */
    icon?: Component
    label: string
    /** The reason under the label, in the caller's words (an upstream message). */
    detail?: string | null
    /** The single action's label; omitted regions offer nothing. */
    action?: string
  }>(),
  {
    busy: false,
    failed: false,
    detail: null,
  },
)
defineEmits<{ action: [] }>()
</script>

<template>
  <div
    class="flex flex-col items-center justify-center gap-2 px-6 py-8 text-center"
    :role="failed ? 'alert' : 'status'"
    :aria-live="failed ? 'assertive' : 'polite'"
    :aria-busy="busy || undefined"
  >
    <IndeterminateSpinner v-if="busy" :size="16" class="text-fg-subtle" />
    <component
      :is="icon ?? CircleX"
      v-else-if="icon || failed"
      :size="20"
      class="text-fg-faint"
      aria-hidden="true"
    />
    <p class="text-chrome text-fg-muted">{{ label }}</p>
    <p
      v-if="detail"
      class="max-w-sm whitespace-pre-line text-[12px] leading-4 text-fg-subtle"
    >
      {{ detail }}
    </p>
    <Button v-if="action" size="sm" class="mt-1" @click="$emit('action')">{{
      action
    }}</Button>
  </div>
</template>
