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
    <!-- Keep the control row in the measured dock, even at the scroll bottom.
         Chips read from the left; the scroll control keeps the right edge. -->
    <div class="flex items-center gap-1.5 pb-2">
      <slot name="chips" />
      <span class="ml-auto inline-flex size-7 shrink-0">
        <Transition
          appear
          :duration="200"
          enter-active-class="transition-opacity duration-200 ease-out"
          leave-active-class="transition-opacity duration-200 ease-out"
          enter-from-class="opacity-0"
          leave-to-class="opacity-0"
        >
          <IconButton
            v-if="showScrollToBottom"
            :icon="ChevronDown"
            variant="solid"
            circle
            aria-label="Scroll to bottom"
            @click="emit('scrollToBottom')"
            @transitionend.stop
          />
        </Transition>
      </span>
    </div>
    <div class="relative z-10">
      <slot />
    </div>
  </div>
</template>
