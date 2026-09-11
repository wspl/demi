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
    <!-- Only task controls contribute to the measured dock height. -->
    <div class="flex justify-start gap-1.5 pb-2 empty:hidden">
      <slot name="chips" />
    </div>
    <!-- Scroll position must not change dock height or transcript padding. -->
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
        class="absolute bottom-full right-0 mb-2 inline-flex"
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
    <div class="relative z-10">
      <slot />
    </div>
  </div>
</template>
