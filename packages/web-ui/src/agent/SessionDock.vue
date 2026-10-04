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
  <!-- What stands over the composer stacks at one 8px step: the card, the chips, the composer.
       A part that is not there takes no room, so the next one moves up by the same step. -->
  <div class="relative">
    <!-- The scroll control floats over the transcript at the dock's top right, so it takes no
         room in the dock; it shows only away from the scroll bottom, where it covers nothing the
         user reads last. -->
    <span class="pointer-events-none absolute bottom-full right-0 mb-2 inline-flex size-7">
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
          class="pointer-events-auto"
          :icon="ChevronDown"
          variant="solid"
          circle
          aria-label="Scroll to bottom"
          @click="emit('scrollToBottom')"
          @transitionend.stop
        />
      </Transition>
    </span>
    <!-- What waits for the user's decision sits over the chips, below the transcript. -->
    <div v-if="$slots.above" class="pb-2">
      <slot name="above" />
    </div>
    <!-- Chips read from the left. A chip renders nothing while it has nothing to say, so the row
         shows only while it holds an element. -->
    <div class="hidden items-center gap-1.5 pb-2 has-[>*]:flex">
      <slot name="chips" />
    </div>
    <div class="relative z-10">
      <slot />
    </div>
  </div>
</template>
