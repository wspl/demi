<script setup lang="ts">
import { computed, type Component } from 'vue'
import { CircleX } from '@lucide/vue'
import Button from './Button.vue'
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import { regionRetry } from './region-retry'
import type { SentenceText, TitleText } from './ui-text'

/**
 * The pane that stands in for a region whose content cannot be shown yet:
 * a loading list, a failed restore, an empty conversation, a missing id.
 *
 * Every region uses this one layout, so a failure reads the same in the
 * session pane, a settings list, the sidebar and a dialog body: a glyph for
 * the kind (a spinner while loading), one sentence, an optional detail line,
 * and at most one action. A failed region's action is Retry, and Retry
 * returns the region to loading in the same frame, before the caller's work
 * starts: the failure never stays on screen while it is retried. There is no
 * third state between the two. A failed region announces itself
 * (`role="alert"`); loading and other regions stay polite.
 */
const props = withDefaults(
  defineProps<{
    /** Loading, failed, or a note in place of the content, such as an empty list. */
    status: 'loading' | 'failed' | 'note'
    /** The glyph above a note's or a failure's sentence; a failure defaults to the error mark. */
    icon?: Component
    label: SentenceText
    /** The reason under a failure's label, in the caller's words (an upstream message). */
    detail?: string | null
    /** What the loading pane says while a Retry runs. */
    loadingLabel?: SentenceText
    /** A note's single action; omitted notes offer nothing. */
    action?: TitleText
    /**
     * A failed region's Retry; without it the failure offers nothing. The
     * region shows its loading pane from the click until the promise it
     * returns settles, or, when it returns none, until the caller's status,
     * label or detail next changes: a caller whose Retry returns nothing
     * flips its own status to loading at once.
     */
    onRetry?: () => unknown
  }>(),
  {
    detail: null,
    loadingLabel: 'Loading…',
  },
)
defineEmits<{ action: [] }>()

const { retrying, retry: run } = regionRetry(() => [props.status, props.label, props.detail])
const shown = computed(() => (retrying.value ? 'loading' : props.status))

function retry(): void {
  run(() => props.onRetry?.())
}
</script>

<template>
  <div
    class="flex flex-col items-center justify-center gap-2 px-6 py-8 text-center"
    :role="shown === 'failed' ? 'alert' : 'status'"
    :aria-live="shown === 'failed' ? 'assertive' : 'polite'"
    :aria-busy="shown === 'loading' || undefined"
  >
    <IndeterminateSpinner v-if="shown === 'loading'" :size="16" class="text-fg-subtle" />
    <component
      :is="icon ?? CircleX"
      v-else-if="icon || shown === 'failed'"
      :size="20"
      class="text-fg-faint"
      aria-hidden="true"
    />
    <p class="text-chrome text-fg-muted">{{ retrying ? loadingLabel : label }}</p>
    <p
      v-if="detail && shown === 'failed'"
      class="max-w-sm whitespace-pre-line text-[12px] leading-4 text-fg-subtle"
    >
      {{ detail }}
    </p>
    <Button v-if="shown === 'failed' && onRetry" size="sm" class="mt-1" @click="retry">Retry</Button>
    <Button v-else-if="shown === 'note' && action" size="sm" class="mt-1" @click="$emit('action')">{{
      action
    }}</Button>
  </div>
</template>
