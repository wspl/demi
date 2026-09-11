<script setup lang="ts">
import { ChevronDown } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'

/**
 * The block at the bottom of the session: an optional session notice, the
 * control row (recovery and panel chips, the scroll-to-bottom button) and the
 * composer. The notice has its own row so the chip row keeps its rhythm.
 */
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
    <div v-if="$slots.notice" class="pb-2">
      <slot name="notice" />
    </div>
    <!-- Keep the control row in the measured dock, even at the scroll bottom. -->
    <div class="flex justify-start gap-1.5 pb-2">
      <slot name="chips" />
      <span class="inline-flex size-7 shrink-0">
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
