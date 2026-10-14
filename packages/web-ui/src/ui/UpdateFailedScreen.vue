<script setup lang="ts">
import { CircleX } from '@lucide/vue'
import Button from './Button.vue'

/**
 * The screen over the whole app once the page loaded itself for the build
 * the backend serves and still got another one, as when a cache in front of
 * the backend keeps an earlier page (`web-application.md` § A page of
 * another build). It says why and offers to load the page again.
 */
defineEmits<{ reload: [] }>()
</script>

<template>
  <div
    class="fixed inset-0 z-[100] flex flex-col items-center justify-center gap-2 bg-surface-base px-6 text-center"
    role="alert"
    aria-live="assertive"
  >
    <CircleX :size="20" class="text-fg-faint" aria-hidden="true" />
    <h1 class="text-[15px] font-medium leading-5 text-fg-emphasis">This Page Could Not Be Updated</h1>
    <p class="max-w-sm text-chrome text-fg-muted">
      Demi has a newer version, but reloading still shows the old page.
    </p>
    <Button size="sm" class="mt-1" @click="$emit('reload')">Reload</Button>
  </div>
</template>
