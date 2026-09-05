<script setup lang="ts">
import { ChevronDown } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'

withDefaults(defineProps<{
  showScrollToBottom?: boolean
}>(), {
  showScrollToBottom: false,
})

const emit = defineEmits<{
  scrollToBottom: []
}>()
</script>

<template>
  <div class="relative">
    <!-- The row stays mounted so the chip's leave fade runs; it hides itself once nothing is left in it. -->
    <div class="flex justify-start gap-1.5 pb-2 empty:hidden">
      <slot name="chips" />
      <Transition
        appear
        :duration="200"
        enter-active-class="transition-opacity duration-200 ease-out"
        leave-active-class="transition-opacity duration-200 ease-out"
        enter-from-class="opacity-0"
        leave-to-class="opacity-0"
      >
        <span
          v-if="showScrollToBottom"
          class="inline-flex"
        >
          <IconButton
            :icon="ChevronDown"
            circle
            aria-label="Scroll to bottom"
            @click="emit('scrollToBottom')"
            @transitionend.stop
          />
        </span>
      </Transition>
    </div>
    <div class="relative z-10">
      <slot />
    </div>
  </div>
</template>
