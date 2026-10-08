<script setup lang="ts">
import { inject, onBeforeUnmount, ref, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { ChevronDown } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import { sessionCoverKey } from './session-cover'

withDefaults(defineProps<{
  showScrollToBottom?: boolean
}>(), {
  showScrollToBottom: false,
})

const emit = defineEmits<{
  scrollToBottom: []
}>()

// What waits for a decision stands over the dock's top and takes no room in it, so a panel the
// dock opened keeps its size and place under it; the surface keeps the transcript clear of it.
const coverRef = ref<HTMLElement>()
const { height: coverHeight } = useElementSize(coverRef, { width: 0, height: 0 }, { box: 'border-box' })
const surfaceCover = inject(sessionCoverKey, null)
watch(coverHeight, (height) => {
  if (surfaceCover) {
    surfaceCover.value = height
  }
}, { immediate: true })
onBeforeUnmount(() => {
  if (surfaceCover) {
    surfaceCover.value = 0
  }
})
</script>

<template>
  <!-- What stands over the composer stacks at one 8px step: the card, the chips, the composer.
       A part that is not there takes no room, so the next one moves up by the same step. -->
  <div class="relative">
    <!-- The scroll control floats over the transcript at the dock's top right, so it takes no
         room in the dock; it shows only away from the scroll bottom, where it covers nothing the
         user reads last. -->
    <span
      class="pointer-events-none absolute right-0 mb-2 inline-flex size-7"
      :style="{ bottom: `calc(100% + ${coverHeight}px)` }"
    >
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
    <!-- What waits for the user's decision sits over the chips, below the transcript, and over the
         lower part of an open panel. -->
    <div v-if="$slots.above" ref="coverRef" class="absolute inset-x-0 bottom-full pb-2">
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
