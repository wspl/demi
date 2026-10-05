<script setup lang="ts">
import { CircleX } from '@lucide/vue'
import Button from './Button.vue'
import IndeterminateSpinner from './IndeterminateSpinner.vue'

/**
 * The screen over the whole app while the backend restarts, and once the
 * page could not load the build the backend serves
 * (`web-application.md` § A page of another build). Nothing under it can be
 * used, since nothing works without the backend; the backend does not know
 * whether it stops for an upgrade or a restart, so the screen does not say
 * which. A failed update offers to load the page again.
 */
defineProps<{
  /** The page loaded itself for the served build and still got another one. */
  failed?: boolean
}>()
defineEmits<{ reload: [] }>()
</script>

<template>
  <div
    class="fixed inset-0 z-[100] flex flex-col items-center justify-center gap-2 bg-surface-base px-6 text-center"
    :role="failed ? 'alert' : 'status'"
    :aria-live="failed ? 'assertive' : 'polite'"
    :aria-busy="!failed || undefined"
  >
    <CircleX v-if="failed" :size="20" class="text-fg-faint" aria-hidden="true" />
    <IndeterminateSpinner v-else :size="16" class="text-fg-subtle" />
    <h1 class="text-[15px] font-medium leading-5 text-fg-emphasis">
      {{ failed ? 'This Page Could Not Be Updated' : 'Demi Is Restarting' }}
    </h1>
    <p class="max-w-sm text-chrome text-fg-muted">
      {{
        failed
          ? 'Demi runs a newer version, but loading the page again still gave this one, as when a cache in front of Demi keeps an earlier page.'
          : 'This page goes on when Demi is back.'
      }}
    </p>
    <Button v-if="failed" size="sm" class="mt-1" @click="$emit('reload')">Reload</Button>
  </div>
</template>
